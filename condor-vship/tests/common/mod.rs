#![allow(dead_code, reason = "each test binary uses a subset")]

use std::{env, io::Write, path::Path};

use condor_vship::Vship;
use v_frame::{
    chroma::ChromaSubsampling,
    frame::{Frame, FrameBuilder},
    pixel::Pixel,
};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;

/// The Vship library, or [`None`] to skip a test when it is not installed.
/// Set `CONDOR_VSHIP_REQUIRED=1` to fail instead of skipping.
pub fn vship() -> Option<Vship> {
    match Vship::find(&[]) {
        Ok(vship) => Some(vship),
        Err(error) if env::var_os("CONDOR_VSHIP_REQUIRED").is_some() => {
            panic!("CONDOR_VSHIP_REQUIRED is set but Vship is unavailable: {error}")
        },
        Err(error) => {
            eprintln!("Skipping, Vship is unavailable: {error}");
            None
        },
    }
}

/// A YUV420 frame with gradients plus deterministic noise of `noise`
/// amplitude (in 8 bit steps)
pub fn frame<T: Pixel>(bit_depth: u8, seed: u32, noise: u32) -> Frame<T> {
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
                let value = ((x * 3 + y * 2 + seed as usize * 5) % 200) as u32 + 24 + jitter;
                let value = value.saturating_sub(noise).min(235) as u16;
                *pixel = T::try_from(value << shift).expect("value fits the bit depth");
            }
        }
    }
    frame
}

/// Write 8 bit frames as a Y4M file
pub fn write_y4m(path: &Path, frames: &[Frame<u8>]) {
    let mut file = std::fs::File::create(path).expect("create y4m");
    writeln!(file, "YUV4MPEG2 W{WIDTH} H{HEIGHT} F24:1 Ip A1:1 C420jpeg").expect("write header");
    for frame in frames {
        file.write_all(b"FRAME\n").expect("write frame header");
        for plane in [Some(&frame.y_plane), frame.u_plane.as_ref(), frame.v_plane.as_ref()]
            .into_iter()
            .flatten()
        {
            for row in plane.rows() {
                file.write_all(row).expect("write row");
            }
        }
    }
}
