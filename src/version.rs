//! Runtime version diagnostics and compatibility checks.
//!
//! The generated bindings (`bindings.rs`) are produced from the pinned PROJ
//! headers, so `PROJ_VERSION_MAJOR/MINOR/PATCH` in that file are the version
//! we **built against**. The library may be *linked* against a different PROJ
//! at runtime (e.g. when a system PROJ is used via `PROJ_DIR`/vcpkg/pkg-config),
//! so we query `proj_info()` at runtime and verify compatibility.

use crate::errors::{ProxiError, Result};
use crate::sys;
use std::ffi::CStr;

/// The PROJ version this crate was generated/built against (from the pinned
/// headers). Runtime must be >= this major.minor for full API compatibility.
pub const BUILT_VERSION_MAJOR: u32 = sys::PROJ_VERSION_MAJOR;
pub const BUILT_VERSION_MINOR: u32 = sys::PROJ_VERSION_MINOR;
pub const BUILT_VERSION_PATCH: u32 = sys::PROJ_VERSION_PATCH;

/// A queryable snapshot of the runtime PROJ version + diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjVersion {
    /// Major version number (runtime).
    pub major: u32,
    /// Minor version number (runtime).
    pub minor: u32,
    /// Patch version number (runtime).
    pub patch: u32,
    /// The version string returned by PROJ (e.g. "Rel. 9.4.1, March 1st, 2024").
    pub release: Option<String>,
    /// The bare version string (e.g. "9.4.1" or "9.4.0").
    pub version: Option<String>,
    /// PROJ's computed search path (the `searchpath` field).
    pub search_path: Option<String>,
    /// The individual search paths (`paths` / `path_count`), if any.
    pub paths: Vec<String>,
}

impl ProjVersion {
    /// Query the runtime PROJ version via `proj_info()`.
    pub fn runtime() -> Self {
        // SAFETY: `proj_info` returns a value struct with static string pointers.
        let info = unsafe { sys::proj_info() };
        let cstr = |p: *const std::ffi::c_char| -> Option<String> {
            if p.is_null() {
                None
            } else {
                // SAFETY: PROJ returns a NUL-terminated static string.
                Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
            }
        };
        let mut paths = Vec::with_capacity(info.path_count);
        if !info.paths.is_null() {
            let mut i = 0;
            // SAFETY: PROJ returns a `path_count`-length array of C strings.
            while i < info.path_count {
                let p = unsafe { *info.paths.add(i) };
                if let Some(s) = cstr(p) {
                    paths.push(s);
                }
                i += 1;
            }
        }
        ProjVersion {
            major: info.major.max(0) as u32,
            minor: info.minor.max(0) as u32,
            patch: info.patch.max(0) as u32,
            release: cstr(info.release),
            version: cstr(info.version),
            search_path: cstr(info.searchpath),
            paths,
        }
    }

    /// Whether the runtime PROJ version is compatible with the built version.
    ///
    /// Compatibility rule: the runtime major must equal the built major, and
    /// the runtime minor must be >= the built minor (PROJ's public C API is
    /// stable within a major; a newer minor is backward compatible). If the
    /// runtime reports 0.0.0 (e.g. an uninitialized/static-link probe), it's
    /// treated as the built version.
    pub fn is_compatible_with_built(&self) -> bool {
        if self.major == BUILT_VERSION_MAJOR && self.minor >= BUILT_VERSION_MINOR {
            return true;
        }
        // A runtime of 0.x suggests `proj_info` couldn't report the version;
        // be lenient but surface it via the diagnostic API rather than hard-failing.
        self.major == 0 && self.minor == 0 && self.patch == 0
    }
}

/// Check the runtime PROJ version against the built version, returning a clear
/// error on a mismatch (e.g. building against PROJ 9 but running PROJ 8).
pub fn check_runtime_compatibility() -> Result<()> {
    let runtime = ProjVersion::runtime();
    if runtime.is_compatible_with_built() {
        Ok(())
    } else {
        Err(ProxiError::VersionMismatch {
            built_major: BUILT_VERSION_MAJOR,
            built_minor: BUILT_VERSION_MINOR,
            built_patch: BUILT_VERSION_PATCH,
            runtime_major: runtime.major,
            runtime_minor: runtime.minor,
            runtime_patch: runtime.patch,
        })
    }
}
