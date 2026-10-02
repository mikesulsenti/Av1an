use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

use av_decoders::{Decoder, DecoderError};
use crossbeam_channel::{Receiver, bounded, unbounded};

use crate::{
    CallbackError,
    ColorDescription,
    Colorspace,
    DecodedFrame,
    Error,
    Metric,
    Scorer,
    VideoFrame,
    Vship,
};

/// Scorers used in parallel when not configured
const DEFAULT_STREAMS: usize = 4;

/// How to score a clip
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub metric:          Metric,
    /// Scorers working in parallel. Temporal metrics always use one.
    pub streams:         usize,
    pub gpu_id:          i32,
    /// Frame rate used by CVVDP, defaults to the clip's
    pub fps:             Option<f64>,
    /// Color of the reference, for values its frames do not carry
    pub reference_color: ColorDescription,
    /// Color of the distorted clip, for values its frames do not carry
    pub distorted_color: ColorDescription,
}

impl Settings {
    #[inline]
    pub fn new(metric: Metric) -> Self {
        Self {
            metric,
            streams: DEFAULT_STREAMS,
            gpu_id: 0,
            fps: None,
            reference_color: ColorDescription::default(),
            distorted_color: ColorDescription::default(),
        }
    }

    pub(crate) fn streams(&self) -> usize {
        if self.metric.is_temporal() {
            1
        } else {
            self.streams.max(1)
        }
    }

    /// Frame rate for CVVDP, sanitized like the Vship VapourSynth plugin does
    pub(crate) fn fps(&self, clip_fps: Option<f64>) -> f32 {
        let fps = self.fps.or(clip_fps).unwrap_or(0.0) as f32;
        if fps > 0.0 && fps <= 100_000.0 {
            fps
        } else {
            60.0
        }
    }

    /// Create a scorer for the formats and colors of the first pair of frames
    pub(crate) fn colorspaces(
        &self,
        reference: &impl VideoFrame,
        distorted: &impl VideoFrame,
    ) -> Result<(Colorspace, Colorspace), Error> {
        Ok((
            Colorspace::new(
                reference.format()?,
                reference.color().or(self.reference_color),
            )?,
            Colorspace::new(
                distorted.format()?,
                distorted.color().or(self.distorted_color),
            )?,
        ))
    }
}

/// Collects scores that arrive out of order and reports them in frame order
pub(crate) struct InOrder<OnScore> {
    pending:  BTreeMap<usize, f64>,
    scores:   Vec<f64>,
    on_score: OnScore,
}

impl<OnScore, E> InOrder<OnScore>
where
    OnScore: FnMut(usize, f64) -> Result<(), E>,
    E: Into<CallbackError>,
{
    pub(crate) fn new(on_score: OnScore) -> Self {
        Self {
            pending: BTreeMap::new(),
            scores: Vec::new(),
            on_score,
        }
    }

    pub(crate) fn push(&mut self, index: usize, score: f64) -> Result<(), Error> {
        self.pending.insert(index, score);
        while let Some(score) = self.pending.remove(&self.scores.len()) {
            (self.on_score)(self.scores.len(), score)
                .map_err(|error| Error::Callback(error.into()))?;
            self.scores.push(score);
        }
        Ok(())
    }

    /// Scores in frame order, failing if any frame before the last is missing
    pub(crate) fn finish(self) -> Result<Vec<f64>, Error> {
        if self.pending.is_empty() {
            Ok(self.scores)
        } else {
            Err(Error::Frame {
                index:   self.scores.len(),
                message: "missing score".to_owned(),
            })
        }
    }
}

/// Record a worker result, keeping the first error and signalling workers to
/// stop
pub(crate) fn record<OnScore, E>(
    result: Result<(usize, f64), Error>,
    in_order: &mut InOrder<OnScore>,
    first_error: &mut Option<Error>,
    aborted: &AtomicBool,
) where
    OnScore: FnMut(usize, f64) -> Result<(), E>,
    E: Into<CallbackError>,
{
    let result = result.and_then(|(index, score)| {
        if first_error.is_none() {
            in_order.push(index, score)
        } else {
            Ok(())
        }
    });
    if let Err(error) = result {
        aborted.store(true, Ordering::Relaxed);
        first_error.get_or_insert(error);
    }
}

impl Vship {
    /// Score pairs of `(reference, distorted)` frames in parallel with
    /// [`Settings::streams`] scorers.
    ///
    /// Frames are pulled from `pairs` on the calling thread, with at most two
    /// pairs per scorer waiting, so memory stays bounded. `on_score` is called
    /// on the calling thread in frame order. Returns the scores in frame
    /// order.
    ///
    /// # Errors
    /// The first error from `pairs`, a scorer or `on_score`.
    /// [`Error::UnsupportedFormat`] is returned before any frame is scored.
    #[inline]
    pub fn score_frames<R, D, Pairs, OnScore, E>(
        &self,
        settings: &Settings,
        pairs: Pairs,
        on_score: OnScore,
    ) -> Result<Vec<f64>, Error>
    where
        R: VideoFrame + Send,
        D: VideoFrame + Send,
        Pairs: IntoIterator<Item = Result<(R, D), Error>>,
        OnScore: FnMut(usize, f64) -> Result<(), E>,
        E: Into<CallbackError>,
    {
        let mut pairs = pairs.into_iter();
        let Some(first) = pairs.next() else {
            return Ok(Vec::new());
        };
        let first = first?;
        let (reference_colorspace, distorted_colorspace) =
            settings.colorspaces(&first.0, &first.1)?;
        let fps = settings.fps(None);
        let streams = settings.streams();

        let aborted = AtomicBool::new(false);
        let mut in_order = InOrder::new(on_score);
        let mut first_error = None;

        thread::scope(|scope| {
            let (work_tx, work_rx) = bounded::<(usize, R, D)>(streams * 2);
            let (result_tx, result_rx) = unbounded();
            for _ in 0..streams {
                let (work_rx, result_tx, aborted): (Receiver<(usize, R, D)>, _, _) =
                    (work_rx.clone(), result_tx.clone(), &aborted);
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
                    for (index, reference, distorted) in work_rx {
                        if aborted.load(Ordering::Relaxed) {
                            break;
                        }
                        let result = scorer
                            .score(&reference, &distorted)
                            .map(|score| (index, score))
                            .map_err(|error| Error::Frame {
                                index,
                                message: error.to_string(),
                            });
                        if result_tx.send(result).is_err() {
                            break;
                        }
                    }
                });
            }
            drop((work_rx, result_tx));

            for (index, pair) in std::iter::once(Ok(first)).chain(pairs).enumerate() {
                for result in result_rx.try_iter() {
                    record(result, &mut in_order, &mut first_error, &aborted);
                }
                if first_error.is_some() {
                    break;
                }
                match pair {
                    // Fails only when every scorer has stopped, their errors
                    // are collected below
                    Ok((reference, distorted)) => {
                        if work_tx.send((index, reference, distorted)).is_err() {
                            break;
                        }
                    },
                    Err(error) => {
                        aborted.store(true, Ordering::Relaxed);
                        first_error = Some(error);
                        break;
                    },
                }
            }
            drop(work_tx);
            for result in result_rx {
                record(result, &mut in_order, &mut first_error, &aborted);
            }
        });

        first_error.map_or_else(|| in_order.finish(), Err)
    }

    /// Decode and score every frame of two [`Decoder`]s, see
    /// [`Self::score_frames`]. Both decoders must have the same number of
    /// frames.
    ///
    /// # Errors
    /// See [`Self::score_frames`], plus decoding errors and
    /// [`Error::Frame`] when one decoder ends before the other.
    #[inline]
    pub fn compare_decoders<OnScore, E>(
        &self,
        settings: &Settings,
        reference: &mut Decoder,
        distorted: &mut Decoder,
        on_score: OnScore,
    ) -> Result<Vec<f64>, Error>
    where
        OnScore: FnMut(usize, f64) -> Result<(), E>,
        E: Into<CallbackError>,
    {
        let frame_rate = reference.get_video_details().frame_rate;
        let settings = Settings {
            fps: settings.fps.or_else(|| {
                (*frame_rate.denom() != 0)
                    .then(|| f64::from(*frame_rate.numer()) / f64::from(*frame_rate.denom()))
            }),
            ..settings.clone()
        };

        let mut index = 0;
        let pairs = std::iter::from_fn(|| {
            let pair = match (DecodedFrame::read(reference), DecodedFrame::read(distorted)) {
                (Ok(reference), Ok(distorted)) => Ok((reference, distorted)),
                (Err(DecoderError::EndOfFile), Err(DecoderError::EndOfFile)) => return None,
                (Err(DecoderError::EndOfFile), Ok(_)) | (Ok(_), Err(DecoderError::EndOfFile)) => {
                    Err(Error::Frame {
                        index,
                        message: "reference and distorted have a different number of frames"
                            .to_owned(),
                    })
                },
                (Err(error), _) | (_, Err(error)) => Err(error.into()),
            };
            index += 1;
            Some(pair)
        });
        self.score_frames(&settings, pairs, on_score)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    #[test]
    fn scores_are_reported_in_frame_order() {
        let mut reported = Vec::new();
        let mut in_order = InOrder::new(|index, score| {
            reported.push((index, score));
            Ok::<_, Infallible>(())
        });
        for (index, score) in [(2, 2.0), (0, 0.0), (3, 3.0), (1, 1.0)] {
            in_order.push(index, score).expect("callback succeeds");
        }
        let scores = in_order.finish().expect("no gaps");
        assert_eq!(scores, vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(reported, vec![(0, 0.0), (1, 1.0), (2, 2.0), (3, 3.0)]);
    }

    #[test]
    fn gaps_are_errors() {
        let mut in_order = InOrder::new(|_, _| Ok::<_, Infallible>(()));
        in_order.push(1, 1.0).expect("callback succeeds");
        assert!(matches!(
            in_order.finish(),
            Err(Error::Frame {
                index: 0,
                ..
            })
        ));
    }

    #[test]
    fn callback_errors_stop_reporting() {
        let mut in_order = InOrder::new(|index, _| if index == 1 { Err("stop") } else { Ok(()) });
        in_order.push(0, 0.0).expect("first frame is accepted");
        assert!(matches!(in_order.push(1, 1.0), Err(Error::Callback(_))));
    }

    #[test]
    fn temporal_metrics_use_one_stream() {
        let mut settings = Settings::new(Metric::CVVDP {
            display_model:     None,
            model_config_json: None,
            resize_to_display: false,
            disable_temporal:  false,
        });
        settings.streams = 8;
        assert_eq!(settings.streams(), 1);

        settings.metric = Metric::CVVDP {
            display_model:     None,
            model_config_json: None,
            resize_to_display: false,
            disable_temporal:  true,
        };
        assert_eq!(settings.streams(), 8);

        settings.metric = Metric::SSIMULACRA2;
        settings.streams = 0;
        assert_eq!(settings.streams(), 1);
    }

    #[test]
    fn fps_is_sanitized() {
        let mut settings = Settings::new(Metric::SSIMULACRA2);
        assert_eq!(settings.fps(Some(24.0)), 24.0);
        assert_eq!(settings.fps(None), 60.0);
        assert_eq!(settings.fps(Some(0.0)), 60.0);
        settings.fps = Some(30.0);
        assert_eq!(settings.fps(Some(24.0)), 30.0);
    }
}
