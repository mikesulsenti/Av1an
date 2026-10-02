//! Vship metrics through its VapourSynth plugin, and natively through
//! [`condor_vship`] when the library is installed.

use std::sync::mpsc::Sender;

use condor_vship::{Metric, Settings, Vship, vapoursynth::plugin_path};
use tracing::debug;
use vapoursynth::{core::CoreRef, node::Node};

use crate::{
    core::sequence::{SequenceCompletion, SequenceStatus, Status},
    vapoursynth::VapourSynthError,
};

pub mod butteraugli;
pub mod cvvdp;
pub mod ssimulacra2;

pub(in crate::vapoursynth::plugins::vship) const NAME: &str = "VapourSynth-HIP";
pub(in crate::vapoursynth::plugins::vship) const ID: &str = condor_vship::vapoursynth::PLUGIN_ID;
pub(in crate::vapoursynth::plugins::vship) const DOCS: &str = "https://codeberg.org/Line-fr/Vship";

/// Score with the native Vship library ([`condor_vship`]) when it is
/// installed, reporting progress and calling `on_frame` in frame order like
/// [`collect_frame_values`](crate::vapoursynth::plugins::MetricPluginFunction::collect_frame_values).
///
/// Returns [`None`] when native Vship is unavailable or cannot read the clips,
/// in which case the VapourSynth plugins should be used.
#[inline]
pub fn native_scores<'core, OnFrame>(
    core: CoreRef<'core>,
    metric: Metric,
    streams: usize,
    reference: &Node<'core>,
    distorted: &Node<'core>,
    progress_tx: &Sender<SequenceStatus>,
    on_frame: OnFrame,
) -> Result<Option<Vec<f64>>, condor_vship::Error>
where
    OnFrame: Fn(usize, &f64) -> Result<(), VapourSynthError>,
{
    let Some(vship) = Vship::global(|| plugin_path(core).into_iter().collect()) else {
        return Ok(None);
    };
    let name = metric.name();
    let total_frames = reference.info().num_frames as u64;
    let settings = Settings {
        streams,
        ..Settings::new(metric)
    };

    let scores = vship.score_nodes(&settings, reference, distorted, |index, score| {
        on_frame(index, &score)?;
        let _ = progress_tx.send(SequenceStatus::Whole(Status::Processing {
            id:         name.to_owned(),
            completion: SequenceCompletion::Frames {
                completed: index as u64 + 1,
                total:     total_frames,
            },
        }));
        Ok::<_, VapourSynthError>(())
    });
    match scores {
        Ok(scores) => {
            let _ = progress_tx.send(SequenceStatus::Whole(Status::Completed {
                id: name.to_owned(),
            }));
            Ok(Some(scores))
        },
        Err(condor_vship::Error::UnsupportedFormat(reason)) => {
            debug!("Native Vship cannot read the clips ({reason}), using the VapourSynth plugin");
            Ok(None)
        },
        Err(error) => Err(error),
    }
}
