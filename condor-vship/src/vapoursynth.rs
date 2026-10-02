//! Scoring VapourSynth frames and nodes

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
};

use crossbeam_channel::unbounded;
use vapoursynth::{
    core::CoreRef,
    format::{ColorFamily, SampleType},
    frame::{Frame as VsFrame, FrameRef},
    node::Node,
    video_info::Property,
};

use crate::{
    CallbackError,
    ColorDescription,
    Error,
    FrameFormat,
    Planes,
    Range,
    Sample,
    Scorer,
    Settings,
    VideoFrame,
    Vship,
    compare::{InOrder, record},
};

/// Identifier of the Vship VapourSynth plugin
pub const PLUGIN_ID: &str = "com.lumen.vship";

/// Path of the loaded Vship VapourSynth plugin, which is also a Vship library
/// usable with [`Vship::find`]
#[inline]
pub fn plugin_path(core: CoreRef<'_>) -> Option<PathBuf> {
    core.get_plugin_by_id(PLUGIN_ID)
        .ok()
        .flatten()
        .and_then(|plugin| plugin.path())
        .and_then(|path| path.to_str().ok())
        .map(PathBuf::from)
}

// SAFETY: The planes are the frame's own planes, described by its format.
unsafe impl VideoFrame for FrameRef<'_> {
    #[inline]
    fn format(&self) -> Result<FrameFormat, Error> {
        let frame: &VsFrame<'_> = self;
        let format = frame.format();
        let rgb = match format.color_family() {
            ColorFamily::YUV => false,
            ColorFamily::RGB => true,
            family => {
                return Err(Error::UnsupportedFormat(format!("{family:?} frames")));
            },
        };
        let sample = match (format.sample_type(), format.bits_per_sample()) {
            (SampleType::Float, 32) => Sample::Float,
            (SampleType::Float, 16) => Sample::Half,
            (SampleType::Integer, bits) => Sample::Integer(bits),
            (SampleType::Float, bits) => {
                return Err(Error::UnsupportedFormat(format!("{bits} bit float frames")));
            },
        };
        Ok(FrameFormat {
            width: frame.width(0),
            height: frame.height(0),
            sample,
            subsampling: (format.sub_sampling_w(), format.sub_sampling_h()),
            rgb,
        })
    }

    #[inline]
    fn color(&self) -> ColorDescription {
        let props = self.props();
        ColorDescription {
            matrix:          props.get_int("_Matrix").ok(),
            transfer:        props.get_int("_Transfer").ok(),
            primaries:       props.get_int("_Primaries").ok(),
            // VapourSynth: 0 = full, 1 = limited
            range:           match props.get_int("_ColorRange") {
                Ok(0) => Some(Range::Full),
                Ok(1) => Some(Range::Limited),
                _ => None,
            },
            chroma_location: props.get_int("_ChromaLocation").ok(),
        }
    }

    #[inline]
    fn planes(&self) -> Planes<'_> {
        let strides = [0, 1, 2]
            .map(|plane| i64::try_from(self.stride(plane)).expect("stride should fit in an i64"));
        // SAFETY: The pointers and strides are the frame's planes.
        unsafe {
            Planes::new(
                [self.data_ptr(0), self.data_ptr(1), self.data_ptr(2)],
                strides,
            )
        }
    }
}

impl Vship {
    /// Score every frame of `distorted` against `reference`.
    ///
    /// Each of the [`Settings::streams`] scorers requests frames from the nodes
    /// itself, so VapourSynth filters run in parallel. `on_score` is called on
    /// the calling thread in frame order. The frame properties of the first
    /// frames describe the color, falling back to [`Settings`]. CVVDP uses the
    /// reference frame rate unless [`Settings::fps`] is set.
    ///
    /// # Errors
    /// [`Error::UnsupportedFormat`] before scoring when Vship cannot read the
    /// clips. Otherwise the first error from VapourSynth, a scorer or
    /// `on_score`.
    #[inline]
    pub fn score_nodes<'core, OnScore, E>(
        &self,
        settings: &Settings,
        reference: &Node<'core>,
        distorted: &Node<'core>,
        on_score: OnScore,
    ) -> Result<Vec<f64>, Error>
    where
        OnScore: FnMut(usize, f64) -> Result<(), E>,
        E: Into<CallbackError>,
    {
        let total_frames = reference.info().num_frames;
        if distorted.info().num_frames != total_frames {
            return Err(Error::Frame {
                index:   total_frames.min(distorted.info().num_frames),
                message: format!(
                    "reference has {total_frames} frames but distorted has {}",
                    distorted.info().num_frames
                ),
            });
        }
        if total_frames == 0 {
            return Ok(Vec::new());
        }

        let get_frame = |node: &Node<'core>, index: usize| {
            node.get_frame(index).map_err(|error| Error::Frame {
                index,
                message: error.to_string(),
            })
        };
        let (reference_colorspace, distorted_colorspace) =
            settings.colorspaces(&get_frame(reference, 0)?, &get_frame(distorted, 0)?)?;
        let fps = settings.fps(match reference.info().framerate {
            Property::Constant(framerate) if framerate.denominator > 0 => {
                Some(framerate.numerator as f64 / framerate.denominator as f64)
            },
            _ => None,
        });
        let streams = settings.streams().min(total_frames);

        let next_index = AtomicUsize::new(0);
        let aborted = AtomicBool::new(false);
        let mut in_order = InOrder::new(on_score);
        let mut first_error = None;

        thread::scope(|scope| {
            let (result_tx, result_rx) = unbounded();
            for _ in 0..streams {
                let result_tx = result_tx.clone();
                let (next_index, aborted, get_frame) = (&next_index, &aborted, &get_frame);
                scope.spawn(move || {
                    let scorer = match Scorer::new(
                        self,
                        &settings.metric,
                        reference_colorspace,
                        distorted_colorspace,
                        fps,
                        settings.gpu_id,
                    ) {
                        Ok(scorer) => scorer,
                        Err(error) => {
                            let _ = result_tx.send(Err(error));
                            return;
                        },
                    };
                    while !aborted.load(Ordering::Relaxed) {
                        // A temporal metric has a single scorer, so it sees
                        // the frames in order
                        let index = next_index.fetch_add(1, Ordering::Relaxed);
                        if index >= total_frames {
                            break;
                        }
                        let result = get_frame(reference, index)
                            .and_then(|reference_frame| {
                                let distorted_frame = get_frame(distorted, index)?;
                                scorer.score(&reference_frame, &distorted_frame).map_err(|error| {
                                    Error::Frame {
                                        index,
                                        message: error.to_string(),
                                    }
                                })
                            })
                            .map(|score| (index, score));
                        if result_tx.send(result).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(result_tx);

            for result in result_rx {
                record(result, &mut in_order, &mut first_error, &aborted);
            }
        });

        first_error.map_or_else(|| in_order.finish(), Err)
    }
}
