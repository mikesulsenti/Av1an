use std::{marker::PhantomData, mem::size_of};

use av_decoders::Decoder;
use v_frame::{chroma::ChromaSubsampling, frame::Frame, pixel::Pixel, plane::Plane};

use crate::{ColorDescription, Error, FrameFormat, Sample};

/// Pointers to the three planes of a frame and their strides in bytes
#[derive(Debug, Clone, Copy)]
pub struct Planes<'frame> {
    pub(crate) pointers: [*const u8; 3],
    pub(crate) strides:  [i64; 3],
    _frame:              PhantomData<&'frame ()>,
}

impl Planes<'_> {
    /// # Safety
    /// Each pointer must point to the first visible sample of its plane, and
    /// with its stride describe a plane of the size implied by the frame's
    /// [`FrameFormat`], valid for reads for the lifetime of [`Planes`].
    #[inline]
    pub unsafe fn new(pointers: [*const u8; 3], strides: [i64; 3]) -> Self {
        Self {
            pointers,
            strides,
            _frame: PhantomData,
        }
    }
}

/// A planar frame Vship can score.
///
/// # Safety
/// [`Self::planes`] must describe planes matching [`Self::format`], see
/// [`Planes::new`]. Vship reads the planes based on the format.
pub unsafe trait VideoFrame {
    /// Layout of the frame
    ///
    /// # Errors
    /// [`Error::UnsupportedFormat`] when Vship cannot read the frame.
    fn format(&self) -> Result<FrameFormat, Error>;

    /// Color description carried by the frame, if any. Values the frame does
    /// not describe are taken from [`crate::Settings`].
    #[inline]
    fn color(&self) -> ColorDescription {
        ColorDescription::default()
    }

    fn planes(&self) -> Planes<'_>;
}

// SAFETY: The planes come from the frame's own buffers, and `format` rejects
// frames whose chroma planes do not match the subsampling.
unsafe impl<T: Pixel> VideoFrame for Frame<T> {
    #[inline]
    fn format(&self) -> Result<FrameFormat, Error> {
        let unsupported = |message: &str| Error::UnsupportedFormat(message.to_owned());
        let subsampling = match self.subsampling {
            ChromaSubsampling::Yuv420 => (1, 1),
            ChromaSubsampling::Yuv422 => (1, 0),
            ChromaSubsampling::Yuv444 => (0, 0),
            ChromaSubsampling::Monochrome => return Err(unsupported("monochrome frames")),
        };
        let bit_depth = self.bit_depth.get();
        let container_bits = size_of::<T>() * 8;
        if (container_bits == 8) != (bit_depth == 8) || usize::from(bit_depth) > container_bits {
            return Err(unsupported(&format!(
                "{bit_depth} bit samples in {container_bits} bit pixels"
            )));
        }

        let (width, height) = (self.y_plane.width(), self.y_plane.height());
        for chroma in [&self.u_plane, &self.v_plane] {
            let chroma = chroma.as_ref().ok_or_else(|| unsupported("missing chroma plane"))?;
            if (chroma.width(), chroma.height())
                != (width >> subsampling.0, height >> subsampling.1)
            {
                return Err(unsupported("chroma planes do not match the subsampling"));
            }
        }

        Ok(FrameFormat {
            width,
            height,
            sample: Sample::Integer(bit_depth),
            subsampling,
            rgb: false,
        })
    }

    #[inline]
    fn planes(&self) -> Planes<'_> {
        let plane = |plane: Option<&Plane<T>>| {
            plane.map_or((std::ptr::null(), 0), |plane| {
                let geometry = plane.geometry();
                (
                    plane.data()[geometry.data_origin()..].as_ptr().cast::<u8>(),
                    i64::try_from(geometry.stride() * size_of::<T>())
                        .expect("stride should fit in an i64"),
                )
            })
        };
        let (y, y_stride) = plane(Some(&self.y_plane));
        let (u, u_stride) = plane(self.u_plane.as_ref());
        let (v, v_stride) = plane(self.v_plane.as_ref());
        // SAFETY: The pointers are the visible origins of the frame's planes
        // and the strides are their row lengths in bytes.
        unsafe { Planes::new([y, u, v], [y_stride, u_stride, v_stride]) }
    }
}

/// A frame read from an [`av_decoders::Decoder`] at its native bit depth
#[derive(Debug, Clone)]
pub enum DecodedFrame {
    U8(Frame<u8>),
    U16(Frame<u16>),
}

impl DecodedFrame {
    /// Read the next frame, using 16 bit pixels above 8 bits
    ///
    /// # Errors
    /// See [`Decoder::read_video_frame`].
    #[inline]
    pub fn read(decoder: &mut Decoder) -> Result<Self, av_decoders::DecoderError> {
        if decoder.get_video_details().bit_depth > 8 {
            decoder.read_video_frame().map(Self::U16)
        } else {
            decoder.read_video_frame().map(Self::U8)
        }
    }
}

// SAFETY: Delegates to the `Frame` implementations.
unsafe impl VideoFrame for DecodedFrame {
    #[inline]
    fn format(&self) -> Result<FrameFormat, Error> {
        match self {
            Self::U8(frame) => frame.format(),
            Self::U16(frame) => frame.format(),
        }
    }

    #[inline]
    fn planes(&self) -> Planes<'_> {
        match self {
            Self::U8(frame) => frame.planes(),
            Self::U16(frame) => frame.planes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use v_frame::frame::FrameBuilder;

    use super::*;

    #[test]
    fn padded_frame_planes_point_at_visible_origin() {
        let mut frame = FrameBuilder::new(64, 32, ChromaSubsampling::Yuv420, 10)
            .luma_padding_left(16)
            .luma_padding_top(8)
            .build::<u16>()
            .expect("valid frame");
        if let Some(pixel) = frame.y_plane.pixel_mut(0, 0) {
            *pixel = 1023;
        }

        let format = frame.format().expect("YUV420P10 is supported");
        assert_eq!(format, FrameFormat {
            width:       64,
            height:      32,
            sample:      Sample::Integer(10),
            subsampling: (1, 1),
            rgb:         false,
        });

        let planes = frame.planes();
        let geometry = frame.y_plane.geometry();
        assert_eq!(planes.strides[0], (geometry.stride() * 2) as i64);
        // SAFETY: The pointer is the visible origin of a u16 plane.
        let first = unsafe { planes.pointers[0].cast::<u16>().read() };
        assert_eq!(first, 1023);
        assert!(planes.pointers.iter().all(|pointer| !pointer.is_null()));
    }

    #[test]
    fn monochrome_is_rejected() {
        let gray = FrameBuilder::new(64, 32, ChromaSubsampling::Monochrome, 8)
            .build::<u8>()
            .expect("valid frame");
        assert!(matches!(gray.format(), Err(Error::UnsupportedFormat(_))));
    }
}
