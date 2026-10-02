//! Rust declarations of the Vship C API (`VshipAPI.h` and `VshipColor.h`).
//!
//! Only the declarations used by Condor are mirrored here. The library itself
//! is never linked: it is loaded at runtime (see [`crate::Vship`]), so users
//! provide and can update their own Vship build.
//!
//! The declarations are derived from the Vship API headers, which their author
//! has made available under the plain MIT license:
//!
//! Copyright (c) 2026, Line
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to
//! deal in the Software without restriction, including without limitation the
//! rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
//! sell copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//! IN THE SOFTWARE.

use std::ffi::{c_char, c_int, c_void};

// C enums are `int` sized on every platform Vship supports, so they are
// declared as `c_int` constants rather than Rust enums. This also keeps values
// added by newer Vship versions from being undefined behavior.

/// `Vship_Exception`
pub type Exception = c_int;
pub const NO_ERROR: Exception = 0;

/// `Vship_Handle`
pub type Handle = *mut c_void;

/// `Vship_StructType`
pub type StructType = c_int;
pub const STRUCT_INIT_SSIMULACRA2_1: StructType = 1;
pub const STRUCT_INIT_BUTTERAUGLI_1: StructType = 2;
pub const STRUCT_INIT_CVVDP_1: StructType = 3;
pub const STRUCT_SCORE_SSIMULACRA2: StructType = 4;
pub const STRUCT_SCORE_BUTTERAUGLI: StructType = 5;
pub const STRUCT_SCORE_CVVDP: StructType = 6;

/// `Vship_Sample_t`
pub const SAMPLE_FLOAT: c_int = 0;
pub const SAMPLE_HALF: c_int = 1;
pub const SAMPLE_UINT8: c_int = 2;
pub const SAMPLE_UINT9: c_int = 3;
pub const SAMPLE_UINT10: c_int = 5;
pub const SAMPLE_UINT12: c_int = 7;
pub const SAMPLE_UINT14: c_int = 9;
pub const SAMPLE_UINT16: c_int = 11;

/// `Vship_Range_t`
pub const RANGE_LIMITED: c_int = 0;
pub const RANGE_FULL: c_int = 1;

/// `Vship_ChromaLocation_t`
pub const CHROMA_LOCATION_LEFT: c_int = 0;
pub const CHROMA_LOCATION_TOP: c_int = 3;

/// `Vship_ColorFamily_t` (deprecated by Vship, the matrix decides instead)
pub const COLOR_FAMILY_YUV: c_int = 0;
pub const COLOR_FAMILY_RGB: c_int = 1;

/// `Vship_YUVMatrix_t`
pub const MATRIX_RGB: c_int = 0;
pub const MATRIX_BT709: c_int = 1;
pub const MATRIX_ST170_M: c_int = 6;
pub const MATRICES: &[c_int] = &[0, 1, 5, 6, 8, 9, 10, 14, 16, 17];

/// `Vship_TransferFunction_t`
pub const TRANSFER_BT709: c_int = 1;
pub const TRANSFERS: &[c_int] = &[1, 4, 5, 6, 7, 8, 13, 16, 17, 18];

/// `Vship_Primaries_t`
pub const PRIMARIES_BT709: c_int = 1;
pub const PRIMARIES: &[c_int] = &[1, 4, 5, 6, 7, 9, 12];

/// `Vship_Version`
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Version {
    pub major:       c_int,
    pub minor:       c_int,
    pub minor_minor: c_int,
    /// `Vship_Backend`: HIP = 0, Cuda = 1, Vulkan = 2
    pub backend:     c_int,
}

/// `Vship_ChromaSubsample_t`
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaSubsample {
    pub subw: c_int,
    pub subh: c_int,
}

/// `Vship_CropRectangle_t`
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CropRectangle {
    pub top:    c_int,
    pub bottom: c_int,
    pub left:   c_int,
    pub right:  c_int,
}

/// `Vship_Colorspace_t`
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colorspace {
    pub width:             i64,
    pub height:            i64,
    pub target_width:      i64,
    pub target_height:     i64,
    pub sample:            c_int,
    pub range:             c_int,
    pub subsampling:       ChromaSubsample,
    pub chroma_location:   c_int,
    pub color_family:      c_int,
    pub yuv_matrix:        c_int,
    pub transfer_function: c_int,
    pub primaries:         c_int,
    pub crop:              CropRectangle,
}

/// `Vship_InitSSIMULACRA2_1`
#[repr(C)]
pub struct InitSsimulacra2 {
    pub struct_type:    StructType,
    pub src_colorspace: Colorspace,
    pub dis_colorspace: Colorspace,
    pub gpu_id:         c_int,
}

/// `Vship_InitButteraugli_1`
#[repr(C)]
pub struct InitButteraugli {
    pub struct_type:          StructType,
    pub src_colorspace:       Colorspace,
    pub dis_colorspace:       Colorspace,
    pub q_norm:               c_int,
    pub intensity_multiplier: f32,
    pub gpu_id:               c_int,
}

/// `Vship_InitCVVDP_1`
#[repr(C)]
pub struct InitCvvdp {
    pub struct_type:       StructType,
    pub src_colorspace:    Colorspace,
    pub dis_colorspace:    Colorspace,
    pub fps:               f32,
    pub resize_to_display: bool,
    pub model_key:         *const c_char,
    pub model_config_json: *const c_char,
    pub gpu_id:            c_int,
}

/// `Vship_ScoreSSIMULACRA2`
#[repr(C)]
pub struct ScoreSsimulacra2 {
    pub struct_type: StructType,
    pub score:       f64,
}

/// `Vship_ScoreButteraugli`
#[repr(C)]
pub struct ScoreButteraugli {
    pub struct_type: StructType,
    pub norm_q:      f64,
    pub norm_3:      f64,
    pub norm_inf:    f64,
    /// Optional distortion map, null when not requested
    pub dstp:        *const u8,
    pub dststride:   i64,
}

/// `Vship_ScoreCVVDP`
#[repr(C)]
pub struct ScoreCvvdp {
    pub struct_type: StructType,
    pub score:       f64,
    /// Optional distortion map, null when not requested
    pub dstp:        *const u8,
    pub dststride:   i64,
}

pub type GetVersion = unsafe extern "C" fn() -> Version;
pub type GpuFullCheck = unsafe extern "C" fn(gpu_id: c_int) -> Exception;
pub type GetErrorMessage =
    unsafe extern "C" fn(exception: Exception, out_message: *mut c_char, len: c_int) -> c_int;
pub type GetDetailedLastErrorHandler =
    unsafe extern "C" fn(handle: Handle, out_message: *mut c_char, len: c_int) -> c_int;
pub type InitHandler =
    unsafe extern "C" fn(handle: *mut Handle, argument: *mut c_void) -> Exception;
pub type FreeHandler = unsafe extern "C" fn(handle: Handle) -> Exception;
pub type ComputeHandler = unsafe extern "C" fn(
    handle: Handle,
    score: *mut c_void,
    srcp1: *const *const u8,
    srcp2: *const *const u8,
    line_size: *const i64,
    line_size2: *const i64,
) -> Exception;
pub type Reset = unsafe extern "C" fn(handle: Handle) -> Exception;
