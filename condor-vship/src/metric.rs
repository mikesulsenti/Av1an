use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr,
};

use crate::{Colorspace, Error, VideoFrame, Vship, ffi};

/// Metric computed by Vship and its options
#[derive(Debug, Clone, PartialEq)]
pub enum Metric {
    SSIMULACRA2,
    /// Scores are the `q_norm` norm when it is set and the infinite norm
    /// otherwise
    BUTTERAUGLI {
        /// Defaults to 2
        q_norm:               Option<u8>,
        /// Defaults to 203
        intensity_multiplier: Option<f64>,
    },
    /// CVVDP is temporal: the score of a frame covers every frame scored
    /// before it by the same [`Scorer`], unless `disable_temporal` is set
    CVVDP {
        /// Defaults to `standard_fhd`
        display_model:     Option<String>,
        /// JSON overriding display model properties
        model_config_json: Option<String>,
        resize_to_display: bool,
        disable_temporal:  bool,
    },
}

impl Metric {
    #[inline]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SSIMULACRA2 => "SSIMULACRA2",
            Self::BUTTERAUGLI {
                ..
            } => "BUTTERAUGLI",
            Self::CVVDP {
                ..
            } => "CVVDP",
        }
    }

    /// Temporal metrics need every frame in order through a single [`Scorer`]
    #[inline]
    pub fn is_temporal(&self) -> bool {
        matches!(self, Self::CVVDP {
            disable_temporal: false,
            ..
        })
    }
}

/// A Vship metric handler, scoring one pair of frames at a time.
///
/// Scorers allocate GPU memory and are tied to the thread that created them.
/// Use several to score frames in parallel.
pub struct Scorer<'vship> {
    vship:     &'vship Vship,
    handle:    ffi::Handle,
    metric:    Metric,
    reference: Colorspace,
    distorted: Colorspace,
}

impl<'vship> Scorer<'vship> {
    /// Create a scorer for frames in the `reference` and `distorted`
    /// colorspaces. `fps` is only used by CVVDP.
    ///
    /// # Errors
    /// [`Error::Metric`] if Vship fails to create the handler, for example
    /// when out of VRAM or given an invalid display model.
    #[inline]
    pub fn new(
        vship: &'vship Vship,
        metric: &Metric,
        reference: Colorspace,
        distorted: Colorspace,
        fps: f32,
        gpu_id: i32,
    ) -> Result<Self, Error> {
        let mut handle: ffi::Handle = ptr::null_mut();
        let exception = match metric {
            Metric::SSIMULACRA2 => {
                let mut init = ffi::InitSsimulacra2 {
                    struct_type: ffi::STRUCT_INIT_SSIMULACRA2_1,
                    src_colorspace: reference.raw,
                    dis_colorspace: distorted.raw,
                    gpu_id,
                };
                // SAFETY: `init` is a valid InitSSIMULACRA2_1 struct.
                unsafe { (vship.init_handler)(&raw mut handle, (&raw mut init).cast::<c_void>()) }
            },
            Metric::BUTTERAUGLI {
                q_norm,
                intensity_multiplier,
            } => {
                let mut init = ffi::InitButteraugli {
                    struct_type: ffi::STRUCT_INIT_BUTTERAUGLI_1,
                    src_colorspace: reference.raw,
                    dis_colorspace: distorted.raw,
                    q_norm: q_norm.map_or(2, c_int::from),
                    intensity_multiplier: intensity_multiplier.unwrap_or(203.0) as f32,
                    gpu_id,
                };
                // SAFETY: `init` is a valid InitButteraugli_1 struct.
                unsafe { (vship.init_handler)(&raw mut handle, (&raw mut init).cast::<c_void>()) }
            },
            Metric::CVVDP {
                display_model,
                model_config_json,
                resize_to_display,
                ..
            } => {
                let c_string = |value: &str, name: &str| {
                    CString::new(value).map_err(|_| Error::Metric {
                        metric:  metric.name(),
                        message: format!("{name} contains a nul byte"),
                    })
                };
                let model_key = c_string(
                    display_model.as_deref().unwrap_or("standard_fhd"),
                    "display model",
                )?;
                let model_config_json = c_string(
                    model_config_json.as_deref().unwrap_or_default(),
                    "model config",
                )?;
                let mut init = ffi::InitCvvdp {
                    struct_type: ffi::STRUCT_INIT_CVVDP_1,
                    src_colorspace: reference.raw,
                    dis_colorspace: distorted.raw,
                    fps,
                    resize_to_display: *resize_to_display,
                    model_key: model_key.as_ptr(),
                    model_config_json: model_config_json.as_ptr(),
                    gpu_id,
                };
                // SAFETY: `init` is a valid InitCVVDP_1 struct and the strings
                // outlive the call.
                unsafe { (vship.init_handler)(&raw mut handle, (&raw mut init).cast::<c_void>()) }
            },
        };
        if exception != ffi::NO_ERROR {
            return Err(Error::Metric {
                metric:  metric.name(),
                message: vship.error_message(exception),
            });
        }

        Ok(Self {
            vship,
            handle,
            metric: metric.clone(),
            reference,
            distorted,
        })
    }

    #[inline]
    pub fn metric(&self) -> &Metric {
        &self.metric
    }

    /// Score `distorted` against `reference`.
    ///
    /// # Errors
    /// [`Error::Metric`] if a frame does not match the scorer's colorspaces or
    /// Vship fails.
    #[inline]
    pub fn score(
        &self,
        reference: &impl VideoFrame,
        distorted: &impl VideoFrame,
    ) -> Result<f64, Error> {
        for (frame, expected, name) in [
            (reference.format()?, self.reference.format, "reference"),
            (distorted.format()?, self.distorted.format, "distorted"),
        ] {
            if frame != expected {
                return Err(self.error(format!(
                    "{name} frame is {frame:?} but the scorer was created for {expected:?}"
                )));
            }
        }
        let reference = reference.planes();
        let distorted = distorted.planes();
        let compute = |score: *mut c_void| {
            // SAFETY: The planes were checked to match the colorspaces the
            // handler was created with and are borrowed for the call. `score`
            // is the score struct of this metric.
            let exception = unsafe {
                (self.vship.compute_handler)(
                    self.handle,
                    score,
                    reference.pointers.as_ptr(),
                    distorted.pointers.as_ptr(),
                    reference.strides.as_ptr(),
                    distorted.strides.as_ptr(),
                )
            };
            self.check(exception)
        };

        match &self.metric {
            Metric::SSIMULACRA2 => {
                let mut score = ffi::ScoreSsimulacra2 {
                    struct_type: ffi::STRUCT_SCORE_SSIMULACRA2,
                    score:       0.0,
                };
                compute((&raw mut score).cast())?;
                Ok(score.score)
            },
            Metric::BUTTERAUGLI {
                q_norm, ..
            } => {
                let mut score = ffi::ScoreButteraugli {
                    struct_type: ffi::STRUCT_SCORE_BUTTERAUGLI,
                    norm_q:      0.0,
                    norm_3:      0.0,
                    norm_inf:    0.0,
                    dstp:        ptr::null(),
                    dststride:   0,
                };
                compute((&raw mut score).cast())?;
                Ok(if q_norm.is_some() {
                    score.norm_q
                } else {
                    score.norm_inf
                })
            },
            Metric::CVVDP {
                disable_temporal, ..
            } => {
                if *disable_temporal {
                    self.reset()?;
                }
                let mut score = ffi::ScoreCvvdp {
                    struct_type: ffi::STRUCT_SCORE_CVVDP,
                    score:       0.0,
                    dstp:        ptr::null(),
                    dststride:   0,
                };
                compute((&raw mut score).cast())?;
                Ok(score.score)
            },
        }
    }

    /// Forget previous frames: clears CVVDP's temporal history and
    /// accumulated score. Does nothing for other metrics.
    ///
    /// # Errors
    /// [`Error::Metric`] if Vship fails.
    #[inline]
    pub fn reset(&self) -> Result<(), Error> {
        if matches!(self.metric, Metric::CVVDP { .. }) {
            // SAFETY: The handle is a valid CVVDP handle.
            self.check(unsafe { (self.vship.reset)(self.handle) })?;
            // SAFETY: The handle is a valid CVVDP handle.
            self.check(unsafe { (self.vship.reset_score)(self.handle) })?;
        }
        Ok(())
    }

    fn check(&self, exception: ffi::Exception) -> Result<(), Error> {
        if exception == ffi::NO_ERROR {
            return Ok(());
        }

        let mut buffer = [0 as c_char; 1024];
        // SAFETY: The handle is valid and the buffer length is passed.
        unsafe {
            (self.vship.get_detailed_last_error_handler)(
                self.handle,
                buffer.as_mut_ptr(),
                buffer.len() as c_int,
            );
        }
        let detailed = buffer_to_string(&buffer);
        Err(self.error(if detailed.is_empty() {
            self.vship.error_message(exception)
        } else {
            detailed
        }))
    }

    fn error(&self, message: String) -> Error {
        Error::Metric {
            metric: self.metric.name(),
            message,
        }
    }
}

impl Drop for Scorer<'_> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: The handle was created by Vship_InitHandler and is freed once.
        unsafe {
            (self.vship.free_handler)(self.handle);
        }
    }
}

pub(crate) fn buffer_to_string(buffer: &[c_char]) -> String {
    // SAFETY: c_char and u8 have the same size and alignment.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len()) };
    CStr::from_bytes_until_nul(bytes)
        .map(|message| message.to_string_lossy().trim().to_owned())
        .unwrap_or_default()
}
