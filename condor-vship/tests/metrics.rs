//! Scores computed by a real Vship library. Skipped when Vship is not
//! installed, see `common::vship`.

mod common;

use std::convert::Infallible;

use condor_vship::{Colorspace, Error, Metric, Scorer, Settings, VideoFrame, Vship};
use v_frame::frame::Frame;

#[cfg(test)]
mod tests {
    use super::*;

    fn butteraugli() -> Metric {
        Metric::BUTTERAUGLI {
            q_norm:               None,
            intensity_multiplier: None,
        }
    }

    fn cvvdp(disable_temporal: bool) -> Metric {
        Metric::CVVDP {
            display_model: None,
            model_config_json: None,
            resize_to_display: false,
            disable_temporal,
        }
    }

    fn scorer<'v>(
        vship: &'v Vship,
        metric: &Metric,
        reference: &impl VideoFrame,
        distorted: &impl VideoFrame,
    ) -> Scorer<'v> {
        Scorer::new(
            vship,
            metric,
            colorspace(reference),
            colorspace(distorted),
            24.0,
            0,
        )
        .expect("scorer")
    }

    fn colorspace(frame: &impl VideoFrame) -> Colorspace {
        Colorspace::new(
            frame.format().expect("supported format"),
            Default::default(),
        )
        .expect("supported colorspace")
    }

    fn pairs(count: u32, noise: u32) -> Vec<(Frame<u8>, Frame<u8>)> {
        (0..count)
            .map(|seed| (common::frame(8, seed, 0), common::frame(8, seed, noise)))
            .collect()
    }

    #[test]
    fn identical_frames_score_perfectly() {
        let Some(vship) = common::vship() else { return };
        let reference = common::frame::<u8>(8, 1, 0);

        let ssimulacra2 = scorer(&vship, &Metric::SSIMULACRA2, &reference, &reference)
            .score(&reference, &reference)
            .expect("score");
        assert!(
            ssimulacra2 > 99.0,
            "SSIMULACRA2 of identical frames: {ssimulacra2}"
        );

        let butteraugli = scorer(&vship, &butteraugli(), &reference, &reference)
            .score(&reference, &reference)
            .expect("score");
        assert!(
            butteraugli < 0.01,
            "Butteraugli of identical frames: {butteraugli}"
        );

        let cvvdp = scorer(&vship, &cvvdp(false), &reference, &reference)
            .score(&reference, &reference)
            .expect("score");
        assert!(cvvdp > 9.99, "CVVDP of identical frames: {cvvdp}");
    }

    #[test]
    fn distortion_lowers_quality() {
        let Some(vship) = common::vship() else { return };
        let reference = common::frame::<u16>(10, 1, 0);
        let slightly = common::frame::<u16>(10, 1, 2);
        let heavily = common::frame::<u16>(10, 1, 12);

        for metric in [Metric::SSIMULACRA2, butteraugli(), cvvdp(true)] {
            let scorer = scorer(&vship, &metric, &reference, &slightly);
            let slight = scorer.score(&reference, &slightly).expect("score");
            let heavy = scorer.score(&reference, &heavily).expect("score");
            // Butteraugli is a distance, the others are qualities
            let ordered = if matches!(metric, Metric::BUTTERAUGLI { .. }) {
                slight < heavy
            } else {
                slight > heavy
            };
            assert!(ordered, "{}: slight {slight}, heavy {heavy}", metric.name());
        }
    }

    #[test]
    fn mixed_bit_depths_are_compared() {
        let Some(vship) = common::vship() else { return };
        let reference = common::frame::<u8>(8, 3, 0);
        // The same picture at 10 bits
        let distorted = common::frame::<u16>(10, 3, 0);
        let score = scorer(&vship, &Metric::SSIMULACRA2, &reference, &distorted)
            .score(&reference, &distorted)
            .expect("score");
        assert!(score > 99.0, "8 bit vs 10 bit of the same picture: {score}");
    }

    #[test]
    fn parallel_scores_match_sequential_scores() {
        let Some(vship) = common::vship() else { return };
        let pairs = pairs(12, 6);

        for metric in [Metric::SSIMULACRA2, butteraugli()] {
            let sequential: Vec<f64> = {
                let scorer = scorer(&vship, &metric, &pairs[0].0, &pairs[0].1);
                pairs
                    .iter()
                    .map(|(reference, distorted)| {
                        scorer.score(reference, distorted).expect("score")
                    })
                    .collect()
            };
            let mut settings = Settings::new(metric.clone());
            settings.streams = 4;
            let mut reported = Vec::new();
            let parallel = vship
                .score_frames(&settings, pairs.iter().cloned().map(Ok), |index, _| {
                    reported.push(index);
                    Ok::<_, Infallible>(())
                })
                .expect("scores");
            assert_eq!(parallel, sequential, "{}", metric.name());
            assert_eq!(reported, (0..pairs.len()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn cvvdp_accumulates_over_frames_unless_temporal_is_disabled() {
        let Some(vship) = common::vship() else { return };
        let mut frames = pairs(6, 0);
        // One bad frame in the middle
        frames[2].1 = common::frame(8, 2, 16);

        let temporal = vship
            .score_frames(
                &Settings::new(cvvdp(false)),
                frames.iter().cloned().map(Ok),
                |_, _| Ok::<_, Infallible>(()),
            )
            .expect("scores");
        let independent = vship
            .score_frames(
                &Settings::new(cvvdp(true)),
                frames.iter().cloned().map(Ok),
                |_, _| Ok::<_, Infallible>(()),
            )
            .expect("scores");

        // Independent scores recover after the bad frame, accumulated ones remember it
        assert!(independent[4] > independent[2]);
        assert!(
            temporal[4] < independent[4],
            "temporal {temporal:?} independent {independent:?}"
        );
    }

    #[test]
    fn compare_decoders_matches_in_memory_scores() {
        let Some(vship) = common::vship() else { return };
        let pairs = pairs(5, 6);
        let directory = tempfile::tempdir().expect("temporary directory");
        let reference_path = directory.path().join("reference.y4m");
        let distorted_path = directory.path().join("distorted.y4m");
        common::write_y4m(
            &reference_path,
            &pairs.iter().map(|(reference, _)| reference.clone()).collect::<Vec<_>>(),
        );
        common::write_y4m(
            &distorted_path,
            &pairs.iter().map(|(_, distorted)| distorted.clone()).collect::<Vec<_>>(),
        );

        let settings = Settings::new(Metric::SSIMULACRA2);
        let expected = vship
            .score_frames(&settings, pairs.iter().cloned().map(Ok), |_, _| {
                Ok::<_, Infallible>(())
            })
            .expect("scores");
        let decoded = vship
            .compare_decoders(
                &settings,
                &mut av_decoders::Decoder::from_file(&reference_path).expect("reference"),
                &mut av_decoders::Decoder::from_file(&distorted_path).expect("distorted"),
                |_, _| Ok::<_, Infallible>(()),
            )
            .expect("scores");

        // Too short distorted clip
        common::write_y4m(
            &distorted_path,
            &pairs.iter().take(3).map(|(_, distorted)| distorted.clone()).collect::<Vec<_>>(),
        );
        let mismatched = vship.compare_decoders(
            &settings,
            &mut av_decoders::Decoder::from_file(&reference_path).expect("reference"),
            &mut av_decoders::Decoder::from_file(&distorted_path).expect("distorted"),
            |_, _| Ok::<_, Infallible>(()),
        );

        assert_eq!(decoded, expected);
        assert!(
            matches!(
                mismatched,
                Err(Error::Frame {
                    index: 3,
                    ..
                })
            ),
            "{mismatched:?}"
        );
    }

    #[test]
    fn errors_stop_scoring() {
        let Some(vship) = common::vship() else { return };
        let settings = Settings::new(Metric::SSIMULACRA2);

        let callback =
            vship.score_frames(&settings, pairs(8, 4).into_iter().map(Ok), |index, _| {
                if index == 3 { Err("stop") } else { Ok(()) }
            });
        assert!(matches!(callback, Err(Error::Callback(_))), "{callback:?}");

        let mut frames: Vec<_> = pairs(4, 4).into_iter().map(Ok).collect();
        frames[2] = Err(Error::Frame {
            index:   2,
            message: "decoder failed".to_owned(),
        });
        let source = vship.score_frames(&settings, frames, |_, _| Ok::<_, Infallible>(()));
        assert!(
            matches!(
                source,
                Err(Error::Frame {
                    index: 2,
                    ..
                })
            ),
            "{source:?}"
        );
    }

    #[test]
    fn format_changes_are_rejected() {
        let Some(vship) = common::vship() else { return };
        let reference = common::frame::<u8>(8, 1, 0);
        let ten_bit = common::frame::<u16>(10, 1, 0);
        let scorer = scorer(&vship, &Metric::SSIMULACRA2, &reference, &reference);
        let changed = scorer.score(&reference, &ten_bit);
        assert!(matches!(changed, Err(Error::Metric { .. })), "{changed:?}");
    }

    #[test]
    fn unloadable_paths_are_reported() {
        let missing = Vship::load("/nonexistent/libvship.so");
        assert!(matches!(missing, Err(Error::Load { .. })), "{missing:?}");
    }
}
