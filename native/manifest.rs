//! Emit `cargo:rustc-*` link directives from a native-superbuild manifest
//! JSON. The schema itself carries no project-specific knowledge, so any
//! future CMake superbuild that writes the same shape can reuse this as-is.

use std::fs;
use std::path::Path;

/// Parse a manifest JSON produced by the superbuild and emit link directives.
pub fn emit_link_directives(manifest_path: &Path) {
    let text = fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display()));
    let m: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid manifest JSON: {e}"));

    let include = m["include_dir"].as_str().unwrap_or("");
    if !include.is_empty() {
        println!("cargo:include={include}");
    }
    if let Some(dirs) = m["library_dirs"].as_array() {
        for d in dirs {
            if let Some(s) = d.as_str() {
                println!("cargo:rustc-link-search=native={s}");
            }
        }
    }
    if let Some(libs) = m["libraries"].as_array() {
        for lib in libs {
            if let Some(s) = lib.as_str() {
                // Names are already exactly what the linker expects (see
                // write-manifest.cmake.in) — don't strip lib/.a/.lib here.
                println!("cargo:rustc-link-lib=static={s}");
            }
        }
    }
    if let Some(fw) = m["frameworks"].as_array() {
        for f in fw {
            if let Some(s) = f.as_str() {
                println!("cargo:rustc-link-lib=framework={s}");
            }
        }
    }
    if let Some(sys) = m["system_libraries"].as_array() {
        for s in sys {
            if let Some(name) = s.as_str() {
                println!("cargo:rustc-link-lib=dylib={name}");
            }
        }
    }
    if let Some(data) = m["data_dir"].as_str() {
        if !data.is_empty() {
            println!("cargo:rustc-env=PROXI_DATA_DIR={data}");
            println!("cargo:data_dir={data}");
        }
    }
}
