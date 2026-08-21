//! Regression tests for PROJ-owned export strings and runtime version metadata.

use proxi::sys;

/// Repeatedly copy PROJ-owned WKT and PROJJSON strings before object destruction.
#[test]
fn wkt_projjson_repeat_alloc_free_loop() {
    // SAFETY: `proj_context_create` returns a new context or null.
    let ctx = unsafe { sys::proj_context_create() };
    assert!(!ctx.is_null(), "proj_context_create failed");
    let ctx_ptr = ctx;

    // A bare `proj_context_create()` has no database configured, so EPSG codes
    // cannot resolve. Point it at the bundled data dir (set by `build.rs` via
    // the `PROXI_BUNDLED_DATA_DIR` env var) so `EPSG:4326` is a real CRS. This
    // mirrors what the safe `Context::configured()` does.
    let data_dir = std::env::var("PROXI_BUNDLED_DATA_DIR")
        .or_else(|_| std::env::var("PROJ_DATA"))
        .expect("PROXI_BUNDLED_DATA_DIR or PROJ_DATA must be set by build.rs/configured()");
    // `proj_context_set_search_paths` takes a NULL-terminated array of `const
    // char*`. Build a single-element array; the CString the pointer addresses
    // lives for the duration of the call.
    let c_data_dir = std::ffi::CString::new(data_dir.as_bytes())
        .expect("bundled data dir contains an interior NUL");
    let search_paths: [*const std::ffi::c_char; 1] = [c_data_dir.as_ptr()];
    // SAFETY: `search_paths` is valid for the call and PROJ copies the paths.
    unsafe {
        sys::proj_context_set_search_paths(ctx_ptr, 1, search_paths.as_ptr());
    }

    // SAFETY: `proj_create` parses the string and returns an owned PJ or null.
    let crs = unsafe { sys::proj_create(ctx_ptr, c"EPSG:4326".as_ptr()) };
    assert!(!crs.is_null(), "proj_create(EPSG:4326) failed");

    // These pointers are owned by `crs` and remain valid until the next export
    // or until `crs` is destroyed. The safe wrapper copies them immediately.
    const ITERATIONS: usize = 1000;

    for iteration in 0..ITERATIONS {
        // SAFETY: `proj_as_wkt` returns an object-owned buffer or null.
        let wkt = unsafe { sys::proj_as_wkt(ctx_ptr, crs, sys::PJ_WKT2_2019, std::ptr::null()) };
        assert!(!wkt.is_null(), "proj_as_wkt returned null at {iteration}");
        let wkt_str = unsafe { std::ffi::CStr::from_ptr(wkt) }
            .to_string_lossy()
            .into_owned();
        assert!(
            !wkt_str.is_empty() && wkt_str.contains("CRS"),
            "wkt content at {iteration}: {wkt_str}"
        );
    }

    for iteration in 0..ITERATIONS {
        // SAFETY: `proj_as_projjson` returns an object-owned buffer or null.
        let json = unsafe { sys::proj_as_projjson(ctx_ptr, crs, std::ptr::null()) };
        assert!(
            !json.is_null(),
            "proj_as_projjson returned null at {iteration}"
        );
        let json_str = unsafe { std::ffi::CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert!(json_str.contains("\"type\""), "json content at {iteration}");
    }

    // SAFETY: `crs` is an owned PJ; `proj_destroy` consumes it.
    unsafe { sys::proj_destroy(crs) };
    // SAFETY: `ctx` is an owned context; `proj_context_destroy` consumes it.
    unsafe { sys::proj_context_destroy(ctx_ptr) };
}

/// The runtime version reported by `proj_info()` must match the bundled build.
#[test]
fn proj_info_reports_runtime_version() {
    // SAFETY: `proj_info()` returns a value struct; no pointers to free.
    let info = unsafe { sys::proj_info() };
    assert_eq!(info.major, sys::PROJ_VERSION_MAJOR as i32);
    assert_eq!(info.minor, sys::PROJ_VERSION_MINOR as i32);
}
