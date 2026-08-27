//! Owned PROJ context wrapper.
//!
//! A `Context` owns a `PJ_CONTEXT*`. PROJ contexts are not thread-safe and
//! must be used on the thread that created them; the `PhantomData<Rc<()>>`
//! marker makes `Context` (and anything containing it) `!Send` / `!Sync`.

use crate::errors::{ProxiError, Result};
use crate::sys;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::ptr::NonNull;

/// The PROJ data locations selected for a context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextDataPaths {
    /// The directory containing the database selected for this context.
    pub data_dir: Option<std::path::PathBuf>,
    /// Directories searched by PROJ, in search order.
    pub search_paths: Vec<std::path::PathBuf>,
    /// The writable directory used for downloaded grids, when configured.
    pub user_data_dir: Option<std::path::PathBuf>,
}

fn resolve_data_dir(options: &crate::options::ContextOptions) -> Result<Option<PathBuf>> {
    let has_db = |dir: &PathBuf| dir.join("proj.db").is_file();

    for path in &options.data_paths {
        if std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).is_err() {
            return Err(ProxiError::ContextConfiguration {
                message: format!("invalid data path {}: contains NUL", path.display()),
            });
        }
    }

    if let Some(database_path) = &options.database_path {
        if database_path.file_name().and_then(|name| name.to_str()) != Some("proj.db")
            || !database_path.is_file()
        {
            return Err(ProxiError::MissingData {
                message: format!(
                    "database path {} is not a valid proj.db file",
                    database_path.display()
                ),
            });
        }
        return Ok(database_path.parent().map(PathBuf::from));
    }

    if !options.data_paths.is_empty() {
        if let Some(chosen) = options.data_paths.iter().find(|dir| has_db(dir)) {
            return Ok(Some(chosen.clone()));
        }
        return Err(ProxiError::MissingData {
            message: format!(
                "none of the configured PROJ data paths contains proj.db: {:?}",
                options.data_paths
            ),
        });
    }
    if let Some(dir) = option_env!("PROXI_DATA_DIR") {
        let dir = PathBuf::from(dir);
        if has_db(&dir) {
            return Ok(Some(dir));
        }
    }
    if let Ok(dir) = std::env::var("PROJ_DATA") {
        let dir = PathBuf::from(dir);
        if has_db(&dir) {
            return Ok(Some(dir));
        }
    }
    Ok(None)
}

fn configure_context(
    context: &Context,
    options: &crate::options::ContextOptions,
) -> Result<ContextDataPaths> {
    #[cfg(not(feature = "network"))]
    if options.network_enabled || options.ca_bundle_path.is_some() {
        return Err(ProxiError::Unsupported {
            feature: "network support is required for network and CA configuration",
        });
    }

    let mut data_dir = resolve_data_dir(options)?;
    let mut search_paths = Vec::new();

    if let Some(user_data_dir) = &options.user_data_dir {
        if options.network_enabled {
            std::fs::create_dir_all(user_data_dir).map_err(|error| {
                ProxiError::ContextConfiguration {
                    message: format!(
                        "create PROJ user data directory {}: {error}",
                        user_data_dir.display()
                    ),
                }
            })?;
        }
        search_paths.push(user_data_dir.clone());
    }
    if let Some(data_dir) = &data_dir {
        if !search_paths.contains(data_dir) {
            search_paths.push(data_dir.clone());
        }
    }
    for path in &options.data_paths {
        if !search_paths.contains(path) {
            search_paths.push(path.clone());
        }
    }
    context.set_search_paths(&search_paths)?;

    if let Some(data_dir) = &data_dir {
        let aux_databases = search_paths
            .iter()
            .filter(|path| *path != data_dir)
            .map(|path| path.join("proj.db"))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        context.set_database_path(&data_dir.join("proj.db"), &aux_databases, &[])?;
    }

    if data_dir.is_none() {
        data_dir = context
            .active_database_path()
            .and_then(|path| path.parent().map(PathBuf::from));
    }

    #[cfg(feature = "network")]
    {
        context.set_network_enabled(options.network_enabled);
        if let Some(ca_bundle_path) = &options.ca_bundle_path {
            context.set_ca_bundle_path(ca_bundle_path)?;
        }
    }

    Ok(ContextDataPaths {
        data_dir,
        search_paths,
        user_data_dir: options.user_data_dir.clone(),
    })
}

/// Owned, thread-bound PROJ context.
pub struct Context {
    raw: NonNull<sys::PJ_CONTEXT>,
    data_paths: ContextDataPaths,
    _not_send_sync: PhantomData<std::rc::Rc<()>>,
}

impl Context {
    /// Create a new PROJ context.
    ///
    /// Runs a runtime PROJ version compatibility check against the version this
    /// crate was built with. A mismatch (e.g. built against PROJ 9 but running
    /// PROJ 8) yields a typed [`ProxiError::VersionMismatch`]. Use
    /// [`check_runtime_compatibility`](crate::version::check_runtime_compatibility)
    /// / [`ProjVersion::runtime`](crate::version::ProjVersion::runtime) for granular control.
    pub fn new() -> Result<Self> {
        let mut context = Self::unconfigured()?;
        let paths = configure_context(&context, &crate::options::ContextOptions::default())?;
        context.data_paths = paths;
        Ok(context)
    }

    fn unconfigured() -> Result<Self> {
        // Reject an incompatible runtime up front: the safe wrappers assume the
        // API of the built version.
        crate::version::check_runtime_compatibility()?;
        // SAFETY: `proj_context_create` returns a new context or null.
        let raw = unsafe { sys::proj_context_create() };
        NonNull::new(raw)
            .map(|raw| Self {
                raw,
                data_paths: ContextDataPaths::default(),
                _not_send_sync: PhantomData,
            })
            .ok_or_else(|| ProxiError::ContextConfiguration {
                message: "proj_context_create returned null".to_string(),
            })
    }

    /// Create a context and apply explicit data, network, and TLS settings.
    pub fn configure(options: &crate::options::ContextOptions) -> Result<Self> {
        let mut context = Self::unconfigured()?;
        let paths = configure_context(&context, options)?;
        context.data_paths = paths;
        Ok(context)
    }

    /// The raw pointer. Only valid while the `Context` is alive.
    pub(crate) fn as_ptr(&self) -> *mut sys::PJ_CONTEXT {
        self.raw.as_ptr()
    }

    fn active_database_path(&self) -> Option<PathBuf> {
        let path = unsafe { sys::proj_context_get_database_path(self.as_ptr()) };
        if path.is_null() {
            return None;
        }
        Some(
            unsafe { std::ffi::CStr::from_ptr(path) }
                .to_string_lossy()
                .into_owned()
                .into(),
        )
    }

    /// Return the effective PROJ data locations selected for this context.
    pub fn data_paths(&self) -> ContextDataPaths {
        self.data_paths.clone()
    }

    /// Return the directory containing the database selected for this context.
    pub fn data_dir(&self) -> Option<std::path::PathBuf> {
        self.data_paths.data_dir.clone()
    }

    /// Create a PROJ object (CRS or coordinate operation) by name / identifier,
    /// e.g. "WGS 84" or "UTM zone 33N", via `proj_create_from_name`.
    ///
    /// Returns `None` if no matching object is found. The `types` slice narrows
    /// the object kinds searched (empty = all). `approximate` enables fuzzy
    /// name matching.
    pub fn crs_from_name(&self, name: &str, approximate: bool) -> Option<crate::crs::Crs<'_>> {
        crate::ffi::create_from_name(self, name, &[], approximate)
            .map(|obj| crate::crs::Crs::from_obj(self, obj))
    }

    /// Set the PROJ data search paths on this context.
    ///
    /// Returns true on success. On failure, returns `Err(())` (PROJ may fail
    /// if a path contains an invalid character, but it typically succeeds).
    pub(crate) fn set_search_paths(&self, paths: &[std::path::PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        // Convert to NUL-terminated C strings.
        let c_paths: Vec<std::ffi::CString> = paths
            .iter()
            .map(|p| {
                std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).map_err(|e| {
                    ProxiError::ContextConfiguration {
                        message: format!("invalid data path {}: {e}", p.display()),
                    }
                })
            })
            .collect::<Result<_>>()?;
        let ptrs: Vec<*const std::ffi::c_char> = c_paths.iter().map(|c| c.as_ptr()).collect();
        // SAFETY: `ptrs` and `c_paths` live for the duration of the call and
        // PROJ copies the paths internally.
        unsafe {
            sys::proj_context_set_search_paths(self.as_ptr(), ptrs.len() as i32, ptrs.as_ptr());
        }
        Ok(())
    }

    /// Get the error code and message for the most recent PROJ call on this context.
    pub(crate) fn errno_message(&self) -> (i32, String) {
        let code = unsafe { sys::proj_context_errno(self.as_ptr()) };
        let message = unsafe { sys::proj_context_errno_string(self.as_ptr(), code) };
        let message = if message.is_null() {
            "unknown PROJ error".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        };
        (code, message)
    }

    /// Set the PROJ database path on this context.
    ///
    /// `aux_paths` and `options` are per the PROJ signature
    /// `proj_context_set_database_path(ctx, dbPath, auxDbPaths, options)`:
    /// each is a NULL-terminated `const char* const*`, or NULL when empty.
    ///
    /// This is hardened:
    /// 1. The `proj.db` file must exist on disk; otherwise a clear
    ///    [`ProxiError::MissingData`] is returned (PROJ would otherwise accept
    ///    the path and fail later when a query runs, which surfaces as a
    ///    confusing "no database found" — failing fast here is much clearer).
    /// 2. After configuring, the active database path is read back via
    ///    `proj_context_get_database_path` and compared to the input, catching
    ///    a silently-ignored configuration.
    pub(crate) fn set_database_path(
        &self,
        db_path: &std::path::Path,
        aux_paths: &[std::path::PathBuf],
        options: &[String],
    ) -> Result<()> {
        // 1. The database file must exist before we tell PROJ to use it.
        if !db_path.is_file() {
            return Err(ProxiError::MissingData {
                message: format!(
                    "proj.db not found at {} (set PROJ_DATA/PROXI_BUNDLED_DATA_DIR or an explicit data_dir)",
                    db_path.display()
                ),
            });
        }

        // Keep the backing CStrings alive for the duration of the call.
        let c_db = match std::ffi::CString::new(db_path.as_os_str().as_encoded_bytes()) {
            Ok(c) => c,
            Err(e) => {
                return Err(ProxiError::ContextConfiguration {
                    message: format!("invalid database path {}: {e}", db_path.display()),
                });
            }
        };
        let c_aux: Vec<std::ffi::CString> = aux_paths
            .iter()
            .map(|p| {
                std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).map_err(|e| {
                    ProxiError::ContextConfiguration {
                        message: format!("invalid auxiliary data path {}: {e}", p.display()),
                    }
                })
            })
            .collect::<Result<_>>()?;
        let mut aux_ptrs: Vec<*const std::ffi::c_char> = c_aux.iter().map(|c| c.as_ptr()).collect();
        // PROJ expects NULL-terminated arrays: append a trailing null pointer.
        aux_ptrs.push(std::ptr::null());
        let c_opts: Vec<std::ffi::CString> = options
            .iter()
            .map(|s| {
                std::ffi::CString::new(s.as_bytes()).map_err(|e| ProxiError::ContextConfiguration {
                    message: format!("invalid database option: {e}"),
                })
            })
            .collect::<Result<_>>()?;
        let mut opt_ptrs: Vec<*const std::ffi::c_char> =
            c_opts.iter().map(|c| c.as_ptr()).collect();
        opt_ptrs.push(std::ptr::null());
        // SAFETY: All CStrings are NUL-terminated and alive for the call; the
        // `aux_ptrs`/`opt_ptrs` vectors are NULL-terminated per PROJ's
        // convention. The returned `c_int` is PROJ's status code.
        unsafe {
            let status = sys::proj_context_set_database_path(
                self.as_ptr(),
                c_db.as_ptr(),
                aux_ptrs.as_ptr(),
                opt_ptrs.as_ptr(),
            );
            if status == 0 {
                let (code, message) = self.errno_message();
                return Err(ProxiError::ContextConfiguration {
                    message: format!(
                        "database path rejected (status {status}, errno {code}): {message}"
                    ),
                });
            }
        }

        // 2. Round-trip: read back the active database path and confirm PROJ
        //    accepted ours (not a placeholder).
        // SAFETY: `proj_context_get_database_path` returns a static NUL-
        // terminated string or null, owned by PROJ.
        let active = unsafe { sys::proj_context_get_database_path(self.as_ptr()) };
        if active.is_null() {
            return Err(ProxiError::ContextConfiguration {
                message: format!(
                    "database path was set but PROJ reports no active database (was {:?})",
                    db_path.display()
                ),
            });
        }
        let active = unsafe { std::ffi::CStr::from_ptr(active) }
            .to_string_lossy()
            .into_owned();
        // Compare using canonicalized forms so `/` vs `\`, redundant `.`
        // segments, and case differences don't cause a false mismatch. If
        // canonicalization fails (e.g. the active path no longer exists), fall
        // back to a case-insensitive string compare on the structural filename.
        let requested_canon = db_path.canonicalize();
        let active_canon = std::path::PathBuf::from(&active).canonicalize();
        let matched = match (requested_canon, active_canon) {
            (Ok(a), Ok(b)) => a == b,
            _ => {
                let req = db_path.to_string_lossy().to_lowercase();
                let act = active.to_lowercase();
                req == act
            }
        };
        if !matched {
            return Err(ProxiError::ContextConfiguration {
                message: format!(
                    "database path mismatch: requested {} but PROJ is using {}",
                    db_path.display(),
                    active
                ),
            });
        }
        Ok(())
    }

    /// Enable or disable network grid downloads on this context.
    #[cfg(feature = "network")]
    pub fn set_network_enabled(&self, enabled: bool) {
        // SAFETY: simple int flag.
        unsafe {
            sys::proj_context_set_enable_network(self.as_ptr(), enabled as i32);
        }
    }

    /// Whether network grid downloads are enabled on this context.
    #[cfg(feature = "network")]
    pub fn network_enabled(&self) -> bool {
        // SAFETY: simple int query.
        unsafe { sys::proj_context_is_network_enabled(self.as_ptr()) != 0 }
    }

    /// The current PROJ network URL endpoint (where grids are downloaded from).
    ///
    /// Returns `None` if PROJ has not set one (a default is used internally).
    pub fn url_endpoint(&self) -> Option<String> {
        // SAFETY: `proj_context_get_url_endpoint` returns a static string or null.
        let ptr = unsafe { sys::proj_context_get_url_endpoint(self.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: `ptr` is a NUL-terminated static string.
            Some(
                unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    /// Set the PROJ network URL endpoint used for grid downloads.
    pub fn set_url_endpoint(&self, url: &str) -> Result<()> {
        let c = std::ffi::CString::new(url.as_bytes()).map_err(|e| {
            ProxiError::ContextConfiguration {
                message: format!("invalid URL endpoint: {e}"),
            }
        })?;
        // SAFETY: valid NUL-terminated URL string; `proj_context_set_url_endpoint`
        // returns void; errors surface via errno.
        unsafe { sys::proj_context_set_url_endpoint(self.as_ptr(), c.as_ptr()) };
        Ok(())
    }

    /// Enable or disable PROJ's on-disk grid download cache.
    pub fn grid_cache_set_enable(&self, enabled: bool) {
        // SAFETY: simple int flag.
        unsafe { sys::proj_grid_cache_set_enable(self.as_ptr(), enabled as i32) };
    }

    /// Set the maximum on-disk grid cache size in megabytes.
    pub fn grid_cache_set_max_size_mb(&self, max_size_mb: i32) {
        // SAFETY: simple int value.
        unsafe { sys::proj_grid_cache_set_max_size(self.as_ptr(), max_size_mb) };
    }

    /// Set the grid cache TTL in seconds.
    pub fn grid_cache_set_ttl_seconds(&self, ttl_seconds: i32) {
        // SAFETY: simple int value.
        unsafe { sys::proj_grid_cache_set_ttl(self.as_ptr(), ttl_seconds) };
    }

    /// Clear the on-disk grid download cache.
    pub fn grid_cache_clear(&self) {
        // SAFETY: no return.
        unsafe { sys::proj_grid_cache_clear(self.as_ptr()) };
    }

    /// Set the CA bundle path used for network requests.
    #[cfg(feature = "network")]
    pub(crate) fn set_ca_bundle_path(&self, path: &std::path::Path) -> Result<()> {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| {
            ProxiError::ContextConfiguration {
                message: format!("invalid CA bundle path {}: {e}", path.display()),
            }
        })?;
        // SAFETY: valid NUL-terminated path. `proj_context_set_ca_bundle_path`
        // returns void in PROJ; errors surface via the context errno.
        unsafe { sys::proj_context_set_ca_bundle_path(self.as_ptr(), c.as_ptr()) };
        Ok(())
    }

    #[cfg(feature = "network")]
    pub(crate) fn download_grid(&self, url: &str) -> Result<()> {
        let url = std::ffi::CString::new(url.as_bytes()).map_err(|e| {
            ProxiError::ContextConfiguration {
                message: format!("invalid grid URL: {e}"),
            }
        })?;
        // SAFETY: URL is NUL-terminated and all callback pointers are null.
        let status = unsafe {
            sys::proj_download_file(self.as_ptr(), url.as_ptr(), 1, None, std::ptr::null_mut())
        };
        if status == 0 {
            let (code, message) = self.errno_message();
            return Err(ProxiError::Transform {
                code,
                message: format!("grid download failed for {url:?}: {message}"),
            });
        }
        Ok(())
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { sys::proj_context_destroy(self.raw.as_ptr()) };
    }
}
