//! Vship metric throughput on synthetic frames.
//!
//! Requires a Vship library and a GPU, see the README. Without one the
//! benchmarks are skipped.

use std::{convert::Infallible, hint::black_box};

use condor_vship::{Colorspace, Metric, Scorer, Settings, VideoFrame, Vship};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use v_frame::{
    chroma::ChromaSubsampling,
    frame::{Frame, FrameBuilder},
    pixel::Pixel,
};

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
const FRAMES: usize = 16;

/// A frame with gradients and deterministic noise of `noise` amplitude
fn frame<T: Pixel>(bit_depth: u8, seed: u32, noise: u32) -> Frame<T> {
    let mut frame = FrameBuilder::new(WIDTH, HEIGHT, ChromaSubsampling::Yuv420, bit_depth)
        .build::<T>()
        .expect("valid frame");
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    let shift = bit_depth - 8;
    for plane in [Some(&mut frame.y_plane), frame.u_plane.as_mut(), frame.v_plane.as_mut()]
        .into_iter()
        .flatten()
    {
        for (y, row) in plane.rows_mut().enumerate() {
            for (x, pixel) in row.iter_mut().enumerate() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let jitter = if noise == 0 { 0 } else { state % (noise * 2) };
                let value = ((x + y) % 220) as u32 + 16 + jitter;
                let value = value.saturating_sub(noise).min(235) as u8;
                *pixel = T::try_from(u16::from(value) << shift).expect("value fits the bit depth");
            }
        }
    }
    frame
}

fn metrics() -> Vec<Metric> {
    vec![
        Metric::SSIMULACRA2,
        Metric::BUTTERAUGLI {
            q_norm:               None,
            intensity_multiplier: None,
        },
        Metric::CVVDP {
            display_model:     None,
            model_config_json: None,
            resize_to_display: false,
            disable_temporal:  false,
        },
    ]
}

fn single_frame(c: &mut Criterion, vship: &Vship) {
    let reference = frame::<u16>(10, 1, 0);
    let distorted = frame::<u16>(10, 1, 6);
    let mut group = c.benchmark_group("single_frame_720p_10bit");
    group.throughput(Throughput::Elements(1));
    for metric in metrics() {
        let settings = Settings::new(metric);
        let scorer = Scorer::new(
            vship,
            &settings.metric,
            Colorspace::new(
                reference.format().expect("supported"),
                settings.reference_color,
            )
            .expect("supported"),
            Colorspace::new(
                distorted.format().expect("supported"),
                settings.distorted_color,
            )
            .expect("supported"),
            24.0,
            0,
        )
        .expect("scorer");
        group.bench_function(settings.metric.name(), |b| {
            b.iter(|| scorer.score(black_box(&reference), black_box(&distorted)).expect("score"));
        });
    }
    group.finish();
}

fn parallel_streams(c: &mut Criterion, vship: &Vship) {
    let pairs: Vec<_> = (0..FRAMES)
        .map(|index| {
            (
                frame::<u8>(8, index as u32, 0),
                frame::<u8>(8, index as u32, 6),
            )
        })
        .collect();
    let mut group = c.benchmark_group("score_frames_720p_8bit");
    group.throughput(Throughput::Elements(FRAMES as u64));
    group.sample_size(10);
    for metric in [Metric::SSIMULACRA2, Metric::BUTTERAUGLI {
        q_norm:               None,
        intensity_multiplier: None,
    }] {
        for streams in [1, 2, 4] {
            let mut settings = Settings::new(metric.clone());
            settings.streams = streams;
            group.bench_with_input(
                BenchmarkId::new(metric.name(), format!("{streams}_streams")),
                &settings,
                |b, settings| {
                    b.iter(|| {
                        vship
                            .score_frames(settings, pairs.iter().cloned().map(Ok), |_, _| {
                                Ok::<_, Infallible>(())
                            })
                            .expect("scores")
                    });
                },
            );
        }
    }
    group.finish();
}

fn benches(c: &mut Criterion) {
    match Vship::find(&[]) {
        Ok(vship) => {
            println!("Benchmarking {vship}");
            single_frame(c, &vship);
            parallel_streams(c, &vship);
        },
        Err(error) => println!("Skipping Vship benchmarks: {error}"),
    }
}

criterion_group!(vship, benches);
criterion_main!(vship);
