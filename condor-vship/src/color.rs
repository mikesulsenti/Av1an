use std::ffi::c_int;

use crate::{Error, ffi};

/// Color range of a clip
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Limited,
    Full,
}

/// Color description of a clip, using ITU-T H.273 code points as VapourSynth
/// and FFmpeg do. Unset values fall back to the assumptions of the Vship
/// VapourSynth plugin: BT.709 (BT.601 below 650 lines), limited range YUV, full
/// range RGB, left chroma location.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorDescription {
    pub matrix:          Option<i64>,
    pub transfer:        Option<i64>,
    pub primaries:       Option<i64>,
    pub range:           Option<Range>,
    /// 0 = left, 1 = center, 2 = top left, 3 = top
    pub chroma_location: Option<i64>,
}

impl ColorDescription {
    /// Fill unset values from `fallback`
    #[inline]
    #[must_use]
    pub fn or(self, fallback: Self) -> Self {
        Self {
            matrix:          self.matrix.or(fallback.matrix),
            transfer:        self.transfer.or(fallback.transfer),
            primaries:       self.primaries.or(fallback.primaries),
            range:           self.range.or(fallback.range),
            chroma_location: self.chroma_location.or(fallback.chroma_location),
        }
    }
}

/// Sample type of a frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// Integer samples with the given bit depth, 9 to 16 bit samples are stored
    /// in 16 bits
    Integer(u8),
    /// 16 bit float
    Half,
    /// 32 bit float
    Float,
}

/// Memory layout of a planar frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFormat {
    pub width:       usize,
    pub height:      usize,
    pub sample:      Sample,
    /// log2 horizontal and vertical chroma subsampling
    pub subsampling: (u8, u8),
    pub rgb:         bool,
}

/// A frame format and color description as understood by Vship
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colorspace {
    pub(crate) format: FrameFormat,
    pub(crate) raw:    ffi::Colorspace,
}

impl Colorspace {
    /// Describe frames of `format` with `color` for Vship. Unset or
    /// unsupported color values use the defaults described on
    /// [`ColorDescription`].
    ///
    /// # Errors
    /// [`Error::UnsupportedFormat`] for sample types or subsampling Vship
    /// cannot read.
    #[inline]
    pub fn new(format: FrameFormat, color: ColorDescription) -> Result<Self, Error> {
        let unsupported = || Error::UnsupportedFormat(format!("{format:?}"));
        let sample = match format.sample {
            Sample::Float => ffi::SAMPLE_FLOAT,
            Sample::Half => ffi::SAMPLE_HALF,
            Sample::Integer(8) => ffi::SAMPLE_UINT8,
            Sample::Integer(9) => ffi::SAMPLE_UINT9,
            Sample::Integer(10) => ffi::SAMPLE_UINT10,
            Sample::Integer(12) => ffi::SAMPLE_UINT12,
            Sample::Integer(14) => ffi::SAMPLE_UINT14,
            Sample::Integer(16) => ffi::SAMPLE_UINT16,
            Sample::Integer(_) => return Err(unsupported()),
        };
        let (sub_w, sub_h) = format.subsampling;
        if sub_w > 2 || sub_h > 2 || (format.rgb && format.subsampling != (0, 0)) {
            return Err(unsupported());
        }

        let supported = |value: Option<i64>, supported: &[c_int]| {
            value
                .and_then(|value| c_int::try_from(value).ok())
                .filter(|value| supported.contains(value))
        };
        let yuv_matrix = if format.rgb {
            ffi::MATRIX_RGB
        } else {
            supported(color.matrix, ffi::MATRICES)
                .filter(|matrix| *matrix != ffi::MATRIX_RGB)
                .unwrap_or(if format.height > 650 {
                    ffi::MATRIX_BT709
                } else {
                    ffi::MATRIX_ST170_M
                })
        };
        // BT.2020 10 and 12 bit (14 and 15) use the BT.709 transfer function
        let transfer = color.transfer.map(|transfer| match transfer {
            14 | 15 => i64::from(ffi::TRANSFER_BT709),
            transfer => transfer,
        });
        let transfer_function = supported(transfer, ffi::TRANSFERS).unwrap_or(ffi::TRANSFER_BT709);
        let primaries = supported(color.primaries, ffi::PRIMARIES).unwrap_or(ffi::PRIMARIES_BT709);
        let range = match color.range {
            Some(Range::Full) => ffi::RANGE_FULL,
            Some(Range::Limited) => ffi::RANGE_LIMITED,
            None if format.rgb => ffi::RANGE_FULL,
            None => ffi::RANGE_LIMITED,
        };
        let chroma_location = color
            .chroma_location
            .and_then(|location| c_int::try_from(location).ok())
            .filter(|location| {
                (ffi::CHROMA_LOCATION_LEFT..=ffi::CHROMA_LOCATION_TOP).contains(location)
            })
            .unwrap_or(ffi::CHROMA_LOCATION_LEFT);

        Ok(Self {
            format,
            raw: ffi::Colorspace {
                width: i64::try_from(format.width).map_err(|_| unsupported())?,
                height: i64::try_from(format.height).map_err(|_| unsupported())?,
                target_width: -1,
                target_height: -1,
                sample,
                range,
                subsampling: ffi::ChromaSubsample {
                    subw: c_int::from(sub_w),
                    subh: c_int::from(sub_h),
                },
                chroma_location,
                color_family: if format.rgb {
                    ffi::COLOR_FAMILY_RGB
                } else {
                    ffi::COLOR_FAMILY_YUV
                },
                yuv_matrix,
                transfer_function,
                primaries,
                crop: ffi::CropRectangle::default(),
            },
        })
    }

    #[inline]
    pub fn format(&self) -> FrameFormat {
        self.format
    }

    /// The colorspace passed to Vship
    #[inline]
    pub fn raw(&self) -> ffi::Colorspace {
        self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yuv420p10(height: usize, color: ColorDescription) -> ffi::Colorspace {
        Colorspace::new(
            FrameFormat {
                width: height * 16 / 9,
                height,
                sample: Sample::Integer(10),
                subsampling: (1, 1),
                rgb: false,
            },
            color,
        )
        .expect("YUV420P10 is supported")
        .raw
    }

    #[test]
    fn untagged_yuv_matches_vship_plugin_defaults() {
        let hd = yuv420p10(1080, ColorDescription::default());
        assert_eq!(hd.sample, ffi::SAMPLE_UINT10);
        assert_eq!(hd.subsampling, ffi::ChromaSubsample {
            subw: 1, subh: 1
        });
        assert_eq!(hd.range, ffi::RANGE_LIMITED);
        assert_eq!(hd.yuv_matrix, ffi::MATRIX_BT709);
        assert_eq!(hd.transfer_function, ffi::TRANSFER_BT709);
        assert_eq!(hd.primaries, ffi::PRIMARIES_BT709);
        assert_eq!(hd.chroma_location, ffi::CHROMA_LOCATION_LEFT);
        assert_eq!((hd.width, hd.height), (1920, 1080));
        assert_eq!((hd.target_width, hd.target_height), (-1, -1));

        let sd = yuv420p10(480, ColorDescription::default());
        assert_eq!(sd.yuv_matrix, ffi::MATRIX_ST170_M);
    }

    #[test]
    fn hdr_description_is_honored() {
        let hdr = yuv420p10(2160, ColorDescription {
            matrix:          Some(9),
            transfer:        Some(16),
            primaries:       Some(9),
            range:           Some(Range::Full),
            chroma_location: Some(2),
        });
        assert_eq!(hdr.yuv_matrix, 9);
        assert_eq!(hdr.transfer_function, 16);
        assert_eq!(hdr.primaries, 9);
        assert_eq!(hdr.range, ffi::RANGE_FULL);
        assert_eq!(hdr.chroma_location, 2);
    }

    #[test]
    fn unspecified_or_unsupported_values_use_defaults() {
        let colorspace = yuv420p10(1080, ColorDescription {
            // Unspecified
            matrix:          Some(2),
            transfer:        Some(2),
            // EBU 3213 is not supported by Vship
            primaries:       Some(22),
            range:           None,
            // Bottom left is not supported by Vship
            chroma_location: Some(4),
        });
        assert_eq!(colorspace.yuv_matrix, ffi::MATRIX_BT709);
        assert_eq!(colorspace.transfer_function, ffi::TRANSFER_BT709);
        assert_eq!(colorspace.primaries, ffi::PRIMARIES_BT709);
        assert_eq!(colorspace.range, ffi::RANGE_LIMITED);
        assert_eq!(colorspace.chroma_location, ffi::CHROMA_LOCATION_LEFT);

        let rgb_matrix_on_yuv = yuv420p10(1080, ColorDescription {
            matrix: Some(0),
            ..Default::default()
        });
        assert_eq!(rgb_matrix_on_yuv.yuv_matrix, ffi::MATRIX_BT709);

        let bt2020_12bit_transfer = yuv420p10(1080, ColorDescription {
            transfer: Some(15),
            ..Default::default()
        });
        assert_eq!(bt2020_12bit_transfer.transfer_function, ffi::TRANSFER_BT709);
    }

    #[test]
    fn description_fallback_fills_unset_values() {
        let frame = ColorDescription {
            matrix: Some(9),
            ..Default::default()
        };
        let configured = ColorDescription {
            matrix: Some(1),
            transfer: Some(16),
            ..Default::default()
        };
        assert_eq!(frame.or(configured), ColorDescription {
            matrix: Some(9),
            transfer: Some(16),
            ..Default::default()
        });
    }

    #[test]
    fn rgb_defaults_to_full_range() {
        let rgb = Colorspace::new(
            FrameFormat {
                width:       1920,
                height:      1080,
                sample:      Sample::Float,
                subsampling: (0, 0),
                rgb:         true,
            },
            ColorDescription::default(),
        )
        .expect("RGBS is supported")
        .raw;
        assert_eq!(rgb.sample, ffi::SAMPLE_FLOAT);
        assert_eq!(rgb.range, ffi::RANGE_FULL);
        assert_eq!(rgb.yuv_matrix, ffi::MATRIX_RGB);
        assert_eq!(rgb.color_family, ffi::COLOR_FAMILY_RGB);
    }

    #[test]
    fn unsupported_formats_are_rejected() {
        let eleven_bit = Colorspace::new(
            FrameFormat {
                width:       1920,
                height:      1080,
                sample:      Sample::Integer(11),
                subsampling: (1, 1),
                rgb:         false,
            },
            ColorDescription::default(),
        );
        assert!(matches!(eleven_bit, Err(Error::UnsupportedFormat(_))));

        let subsampled_rgb = Colorspace::new(
            FrameFormat {
                width:       1920,
                height:      1080,
                sample:      Sample::Integer(8),
                subsampling: (1, 1),
                rgb:         true,
            },
            ColorDescription::default(),
        );
        assert!(matches!(subsampled_rgb, Err(Error::UnsupportedFormat(_))));
    }
}
