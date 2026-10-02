//! Native [Vship](https://codeberg.org/Line-fr/Vship) GPU metrics for Condor.
//!
//! Computes SSIMULACRA2, Butteraugli and CVVDP scores by calling the Vship
//! library directly instead of going through its VapourSynth plugin.
//!
//! Vship is **loaded at runtime, never linked**. It is an optional, user
//! provided library that can be updated without rebuilding Condor; when it is
//! missing, callers are expected to fall back to the VapourSynth plugins. See
//! [`Vship::find`] for where the library is searched for.
//!
//! Frames can come from [`av_decoders`] (the same decoders Condor uses) or,
//! with the `vapoursynth` feature, from VapourSynth frames and nodes.
//!
//! ```no_run
//! use condor_vship::{Metric, Settings, Vship};
//!
//! let vship = Vship::find(&[])?;
//! let mut reference = av_decoders::Decoder::from_file("reference.y4m")?;
//! let mut distorted = av_decoders::Decoder::from_file("distorted.y4m")?;
//! let scores = vship.compare_decoders(
//!     &Settings::new(Metric::SSIMULACRA2),
//!     &mut reference,
//!     &mut distorted,
//!     |index, score| {
//!         println!("frame {index}: {score}");
//!         Ok::<_, std::convert::Infallible>(())
//!     },
//! )?;
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```

pub mod ffi;

mod color;
mod compare;
mod frame;
mod library;
mod metric;
#[cfg(feature = "vapoursynth")]
pub mod vapoursynth;

use std::path::PathBuf;

pub use color::{ColorDescription, Colorspace, FrameFormat, Range, Sample};
pub use compare::Settings;
pub use frame::{DecodedFrame, Planes, VideoFrame};
pub use library::{LIBRARY_NAMES, LIBRARY_PATH_VARIABLE, Vship};
pub use metric::{Metric, Scorer};

/// Boxed error returned by a score callback
pub type CallbackError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Native Vship is disabled by {LIBRARY_PATH_VARIABLE}")]
    Disabled,
    #[error("Vship library not found, tried: {}", tried.join(", "))]
    NotFound { tried: Vec<String> },
    #[error("Failed to load Vship from {}: {message}", path.display())]
    Load { path: PathBuf, message: String },
    /// The frames are in a format Vship cannot read
    #[error("Format not supported by Vship: {0}")]
    UnsupportedFormat(String),
    #[error("Vship {metric} failed: {message}")]
    Metric {
        metric:  &'static str,
        message: String,
    },
    #[error("Frame {index}: {message}")]
    Frame { index: usize, message: String },
    #[error(transparent)]
    Decoder(#[from] av_decoders::DecoderError),
    #[error(transparent)]
    Callback(CallbackError),
}
