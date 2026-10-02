use std::{
    env,
    ffi::{c_char, c_int},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use libloading::Library;
use tracing::debug;

use crate::{Error, ffi, metric::buffer_to_string};

/// Environment variable overriding the Vship library path. `none` disables
/// native Vship.
pub const LIBRARY_PATH_VARIABLE: &str = "CONDOR_VSHIP";

/// File names the Vship library is searched for
#[cfg(windows)]
pub const LIBRARY_NAMES: &[&str] = &["vship.dll", "libvship.dll"];
/// File names the Vship library is searched for
#[cfg(target_os = "macos")]
pub const LIBRARY_NAMES: &[&str] = &["libvship.dylib"];
/// File names the Vship library is searched for
#[cfg(not(any(windows, target_os = "macos")))]
pub const LIBRARY_NAMES: &[&str] = &["libvship.so"];

/// Oldest Vship major version with the generic handler API
const MINIMUM_MAJOR_VERSION: c_int = 5;

static GLOBAL: OnceLock<Option<Vship>> = OnceLock::new();

/// A loaded Vship library
pub struct Vship {
    path: PathBuf,
    version: ffi::Version,
    get_error_message: ffi::GetErrorMessage,
    pub(crate) get_detailed_last_error_handler: ffi::GetDetailedLastErrorHandler,
    pub(crate) init_handler: ffi::InitHandler,
    pub(crate) free_handler: ffi::FreeHandler,
    pub(crate) compute_handler: ffi::ComputeHandler,
    pub(crate) reset: ffi::Reset,
    pub(crate) reset_score: ffi::Reset,
    // Keeps the function pointers above valid
    _library: Library,
}

impl std::fmt::Debug for Vship {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vship")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Vship {
    /// Find and load Vship, searching in order:
    ///
    /// 1. The path in the `CONDOR_VSHIP` environment variable. `none` disables
    ///    native Vship.
    /// 2. Next to the running executable.
    /// 3. The platform library search path (`PATH` on Windows,
    ///    `LD_LIBRARY_PATH` and the system library directories on Linux).
    /// 4. `extra_candidates`, such as the path of the Vship VapourSynth plugin,
    ///    which is the same library.
    ///
    /// The first library that loads, is Vship 5 or newer, and passes Vship's
    /// check of GPU 0 is used.
    #[inline]
    pub fn find(extra_candidates: &[PathBuf]) -> Result<Self, Error> {
        let candidates = match env::var_os(LIBRARY_PATH_VARIABLE) {
            Some(path) if path.eq_ignore_ascii_case("none") => return Err(Error::Disabled),
            Some(path) => vec![PathBuf::from(path)],
            None => Self::default_candidates()
                .into_iter()
                .chain(extra_candidates.iter().cloned())
                .collect(),
        };

        let mut tried = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            match Self::load(&candidate) {
                Ok(vship) => return Ok(vship),
                Err(error) => {
                    debug!("{error}");
                    tried.push(error.to_string());
                },
            }
        }
        Err(Error::NotFound {
            tried,
        })
    }

    /// The process-wide Vship library, found with [`Self::find`] on first use.
    /// [`None`] when no usable library was found.
    ///
    /// `extra_candidates` is only called on first use.
    #[inline]
    pub fn global(extra_candidates: impl FnOnce() -> Vec<PathBuf>) -> Option<&'static Self> {
        GLOBAL
            .get_or_init(|| match Self::find(&extra_candidates()) {
                Ok(vship) => {
                    debug!("Using native {vship}");
                    Some(vship)
                },
                Err(error) => {
                    debug!("Native Vship unavailable: {error}");
                    None
                },
            })
            .as_ref()
    }

    /// Library locations searched by default, in order of preference
    #[inline]
    pub fn default_candidates() -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(directory) =
            env::current_exe().ok().and_then(|path| path.parent().map(Path::to_path_buf))
        {
            candidates.extend(
                LIBRARY_NAMES
                    .iter()
                    .map(|name| directory.join(name))
                    .filter(|path| path.exists()),
            );
        }
        // Bare names are resolved by the platform library search path
        candidates.extend(LIBRARY_NAMES.iter().map(PathBuf::from));
        candidates
    }

    /// Load Vship from `path`
    #[inline]
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let error = |message: String| Error::Load {
            path: path.to_path_buf(),
            message,
        };
        // libloading keeps the system's reason in the error source
        let library_error = |library_error: libloading::Error| {
            error(std::error::Error::source(&library_error).map_or_else(
                || library_error.to_string(),
                |source| format!("{library_error}: {source}"),
            ))
        };

        // SAFETY: Loading runs the library's initialization routines, which
        // only set up Vship's own state.
        let library = unsafe { Library::new(path) }.map_err(library_error)?;
        // SAFETY: Each symbol is declared with its signature from VshipAPI.h
        // and the library is stored alongside the function pointers.
        unsafe {
            let get_version: ffi::GetVersion =
                *library.get(b"Vship_GetVersion\0").map_err(library_error)?;
            let version = get_version();
            if version.major < MINIMUM_MAJOR_VERSION {
                return Err(error(format!(
                    "Vship {}.{}.{} is too old, version {MINIMUM_MAJOR_VERSION} or newer is \
                     required",
                    version.major, version.minor, version.minor_minor
                )));
            }
            let gpu_full_check: ffi::GpuFullCheck =
                *library.get(b"Vship_GPUFullCheck\0").map_err(library_error)?;
            let vship = Self {
                path: path.to_path_buf(),
                version,
                get_error_message: *library
                    .get(b"Vship_GetErrorMessage\0")
                    .map_err(library_error)?,
                get_detailed_last_error_handler: *library
                    .get(b"Vship_GetDetailedLastErrorHandler\0")
                    .map_err(library_error)?,
                init_handler: *library.get(b"Vship_InitHandler\0").map_err(library_error)?,
                free_handler: *library.get(b"Vship_FreeHandler\0").map_err(library_error)?,
                compute_handler: *library.get(b"Vship_ComputeHandler\0").map_err(library_error)?,
                reset: *library.get(b"Vship_Reset\0").map_err(library_error)?,
                reset_score: *library.get(b"Vship_ResetScore\0").map_err(library_error)?,
                _library: library,
            };

            let exception = gpu_full_check(0);
            if exception != ffi::NO_ERROR {
                return Err(error(format!(
                    "GPU 0 cannot run Vship: {}",
                    vship.error_message(exception)
                )));
            }
            Ok(vship)
        }
    }

    /// Path the library was loaded from
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[inline]
    pub fn version(&self) -> ffi::Version {
        self.version
    }

    /// Name of the GPU backend the library was built for
    #[inline]
    pub fn backend(&self) -> &'static str {
        match self.version.backend {
            0 => "HIP",
            1 => "CUDA",
            2 => "Vulkan",
            _ => "unknown backend",
        }
    }

    pub(crate) fn error_message(&self, exception: ffi::Exception) -> String {
        let mut buffer = [0 as c_char; 1024];
        // SAFETY: The buffer length is passed and Vship truncates to it.
        unsafe {
            (self.get_error_message)(exception, buffer.as_mut_ptr(), buffer.len() as c_int);
        }
        buffer_to_string(&buffer)
    }
}

impl std::fmt::Display for Vship {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ffi::Version {
            major,
            minor,
            minor_minor,
            ..
        } = self.version;
        write!(
            f,
            "Vship {major}.{minor}.{minor_minor} ({}) from {}",
            self.backend(),
            self.path.display()
        )
    }
}
