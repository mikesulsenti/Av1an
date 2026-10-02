//! Native scores of VapourSynth nodes compared with the Vship VapourSynth
//! plugin. Skipped when Vship or its plugin is not installed.
#![cfg(feature = "vapoursynth")]

mod common;

use std::convert::Infallible;

use condor_vship::{Metric, Settings, Vship, vapoursynth::plugin_path};
use vapoursynth::{node::Node, vsscript::Environment};

/// Outputs 0 and 1 are a 640x360 reference and a blurred copy. The Vship
/// plugin is loaded from `CONDOR_VSHIP` when it is not autoloaded.
const SCRIPT: &str = r#"
import os
import random
import vapoursynth as vs
core = vs.core

if not hasattr(core, "vship") and os.environ.get("CONDOR_VSHIP"):
    core.std.LoadPlugin(os.environ["CONDOR_VSHIP"])

def mosaic(seed):
    rng = random.Random(seed)
    def block():
        color = [rng.randint(16, 235), rng.randint(16, 240), rng.randint(16, 240)]
        return core.std.BlankClip(format=vs.YUV420P8, width=40, height=40, length=1, color=color)
    return core.std.StackVertical([core.std.StackHorizontal([block() for _ in range(16)]) for _ in range(9)])

reference = core.std.Splice([mosaic(seed) for seed in range(6)])
reference = core.std.AssumeFPS(reference, fpsnum=24, fpsden=1)
distorted = core.std.BoxBlur(reference, hradius=2, vradius=2)
reference.set_output(0)
distorted.set_output(1)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Scores of the Vship VapourSynth plugin, read from frame properties
    fn plugin_scores(
        environment: &Environment,
        function: &str,
        property: &str,
    ) -> Option<Vec<f64>> {
        let core = environment.get_core().expect("core");
        let plugin = core.get_plugin_by_id(condor_vship::vapoursynth::PLUGIN_ID).ok()??;
        let (reference, _) = environment.get_output(0).expect("reference");
        let (distorted, _) = environment.get_output(1).expect("distorted");
        let mut arguments = vapoursynth::map::OwnedMap::new(vapoursynth::api::API::get()?);
        arguments.set_node("reference", &reference).expect("set reference");
        arguments.set_node("distorted", &distorted).expect("set distorted");
        let result = plugin.invoke(function, &arguments).expect("invoke");
        let node: Node<'_> = result.get_video_node("clip").expect("clip");
        Some(
            (0..node.info().num_frames)
                .map(|index| {
                    node.get_frame(index)
                        .expect("frame")
                        .props()
                        .get_float(property)
                        .expect("score property")
                })
                .collect(),
        )
    }

    #[test]
    fn native_scores_match_the_vapoursynth_plugin() {
        let Some(_) = common::vship() else { return };
        let environment = Environment::from_script(SCRIPT).expect("script");
        let core = environment.get_core().expect("core");
        let vship = Vship::find(&plugin_path(core).into_iter().collect::<Vec<_>>()).expect("vship");
        let (reference, _) = environment.get_output(0).expect("reference");
        let (distorted, _) = environment.get_output(1).expect("distorted");

        for (metric, function, property) in [
            (Metric::SSIMULACRA2, "SSIMULACRA2", "_SSIMULACRA2"),
            (
                Metric::BUTTERAUGLI {
                    q_norm:               None,
                    intensity_multiplier: None,
                },
                "BUTTERAUGLI",
                "_BUTTERAUGLI_INFNorm",
            ),
            (
                Metric::CVVDP {
                    display_model:     None,
                    model_config_json: None,
                    resize_to_display: false,
                    disable_temporal:  false,
                },
                "CVVDP",
                "_CVVDP",
            ),
        ] {
            let Some(expected) = plugin_scores(&environment, function, property) else {
                eprintln!("Skipping, the Vship VapourSynth plugin is not loaded");
                return;
            };
            let native = vship
                .score_nodes(&Settings::new(metric), &reference, &distorted, |_, _| {
                    Ok::<_, Infallible>(())
                })
                .expect("native scores");
            eprintln!("{function}: native {native:?}\n{function}: plugin {expected:?}");
            assert_eq!(native.len(), expected.len());
            for (native, expected) in native.iter().zip(&expected) {
                // The plugin converts to RGB with zimg first while Vship converts
                // internally, so scores differ very slightly
                let relative = (native - expected).abs() / expected.abs().max(1.0);
                assert!(
                    relative < 0.001,
                    "{function}: native {native} plugin {expected} differ by {relative}"
                );
            }
        }
    }

    #[test]
    fn unsupported_formats_are_reported_before_scoring() {
        let Some(vship) = common::vship() else { return };
        let environment = Environment::from_script(
            "import vapoursynth as vs\nvs.core.std.BlankClip(format=vs.GRAY8, \
             length=2).set_output(0)",
        )
        .expect("script");
        let (gray, _) = environment.get_output(0).expect("gray");
        let mut called = false;
        let result =
            vship.score_nodes(&Settings::new(Metric::SSIMULACRA2), &gray, &gray, |_, _| {
                called = true;
                Ok::<_, Infallible>(())
            });
        assert!(
            matches!(result, Err(condor_vship::Error::UnsupportedFormat(_))),
            "{result:?}"
        );
        assert!(!called);
    }
}
