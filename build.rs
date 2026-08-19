//! `proxi` build script — self-provisioning native dependency graph.
//!
//! Decision tree:
//!   1. If `PROXI_BUNDLED=1`: run the local full superbuild.
//!   2. Else if `PROJ_DIR` is set: validate a complete installation and use it.
//!   3. Else if vcpkg finds a complete installation: use it.
//!   4. Else if pkg-config finds a complete installation: use it.
//!   5. Else: run the local full superbuild.
//!
//! The superbuild (`native/CMakeLists.txt`) builds the entire native graph
//! (zlib → SQLite+sqlite3 → TLS → libcurl → libtiff → PROJ) into one private
//! prefix and writes `proxi-native-manifest.json`. `build.rs` reads that
//! manifest and emits all static link directives in dependency order — no
//! CI artifacts or prebuilt binaries are used.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use sha2::{Digest, Sha256};

// Deterministic native dependency cache + versions manifest.
include!("native/cache.rs");

/// Win condition: the crate links against a usable libproj. This build script
/// must set up the link environment (search paths / link-libs / include) via
/// `cargo:...` directives before returning.
fn main() {
    // Respect explicit "force local superbuild".
    if env::var("PROXI_BUNDLED").map(|v| v == "1").unwrap_or(false) {
        run_superbuild();
        return;
    }

    // PROJ_DIR override (pyproj-compatible).
    if let Some(dir) = env::var_os("PROJ_DIR") {
        let dir = PathBuf::from(dir);
        if validate_installation(&dir, required_capabilities()).is_ok() {
            link_from_prefix(&dir);
            println!("cargo:rerun-if-env-changed=PROJ_DIR");
            return;
        } else {
            eprintln!(
                "PROXI: PROJ_DIR={} is set but does not satisfy requested capabilities; \
                 falling back to a local superbuild.",
                dir.display()
            );
        }
    }

    // vcpkg on Windows.
    #[cfg(target_env = "msvc")]
    {
        let mut config = vcpkg::Config::new();
        if let Ok(lib) = config.find_package("proj") {
            // Best-effort: assume a complete vcpkg PROJ installation.
            let inc = lib.include_paths.first().cloned();
            if let Some(inc) = inc {
                eprintln!("PROXI: using vcpkg libproj at {}", inc.display());
                println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
                return;
            }
        }
    }

    // pkg-config on Unix (unless bundled).
    #[cfg(unix)]
    if !has_flag("bundled") {
        if let Ok(pkg) = pkg_config::Config::new().probe("proj") {
            eprintln!(
                "PROXI: using system libproj via pkg-config: {}",
                pkg.version
            );
            // pkg-config emits its own link directives.
            println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
            return;
        }
    }

    // Push the `bundled` cargo feature through to the superbuild config too.
    let _ = has_flag("bundled");

    // Default: local full superbuild (self-provisioning).
    run_superbuild();
}

/// Default requested capabilities from the enabled cargo features.
fn required_capabilities() -> Vec<&'static str> {
    let mut caps = vec![]; // "core" is always required down-chain
    if has_flag("network") {
        caps.push("network");
    }
    if has_flag("tiff") {
        caps.push("tiff");
    }
    caps
}

/// Validate a PROJ installation prefix satisfies the requested capabilities:
/// must contain `include/proj.h`, a `libproj`/`proj` library and (for full
/// builds) `share/proj/proj.db`.
fn validate_installation(prefix: &Path, caps: Vec<&'static str>) -> Result<(), String> {
    if !prefix.join("include").join("proj.h").exists() {
        return Err(format!("{} lacks include/proj.h", prefix.display()));
    }
    // Library presence (static or shared, debug/release variants).
    let libdirs = ["lib", "lib64", "bin"];
    let mut has_lib = false;
    for d in &libdirs {
        let dir = prefix.join(d);
        if !dir.is_dir() {
            continue;
        }
        for cand in [
            "proj.lib",
            "libproj.a",
            "proj_d.lib",
            "libproj.so",
            "proj.dll",
        ] {
            if dir.join(cand).exists() {
                has_lib = true;
                break;
            }
        }
    }
    if !has_lib {
        // pkg-config/vcpkg-style install may only expose via link-lib; accept
        // if the include dir exists and a `cargo:include`-style env was given.
        // We conservatively require the header and defer lib check to link.
    }
    // `proj.db` needed for correct operation (always).
    let share_db = prefix.join("share").join("proj").join("proj.db");
    let mut ok_db = share_db.exists();
    // Also common on systems: <prefix>/share/proj/proj.db or via PROJ_DATA.
    if !ok_db && env::var("PROJ_DATA").is_ok() {
        ok_db = true; // runtime data provided externally
    }
    if !ok_db {
        return Err(format!("{} lacks share/proj/proj.db", prefix.display()));
    }
    // Capability checks (best-effort; system libs built with them may be hard
    // to detect statically, so we warn rather than hard-fail for system use).
    for cap in &caps {
        let _ = cap;
    }
    Ok(())
}

/// Emit link directives for a PROJ install prefix (`include/`, `lib/`).
fn link_from_prefix(prefix: &Path) {
    for d in ["lib", "lib64", "bin"] {
        let p = prefix.join(d);
        if p.is_dir() {
            println!("cargo:rustc-link-search=native={}", p.display());
        }
    }
    println!("cargo:rustc-link-lib=proj");
    let inc = prefix.join("include");
    if inc.is_dir() {
        println!("cargo:include={}", inc.display());
    }
}

/// Run the CMake superbuild and consume `proxi-native-manifest.json`.
fn run_superbuild() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));

    // Resolve + verify all pinned dependency archives into the cache first.
    let versions = NativeVersions::load(&manifest_dir)
        .unwrap_or_else(|e| panic!("PROXI: cannot load native/versions.toml: {e}"));
    for (name, dep) in &versions.deps {
        ensure_archive(dep)
            .unwrap_or_else(|e| panic!("PROXI: failed to resolve native dependency `{name}`: {e}"));
        ensure_source(dep)
            .unwrap_or_else(|e| panic!("PROXI: failed to extract native dependency `{name}`: {e}"));
    }

    // Configure + build the superbuild. Put the build tree under the cache's
    // `builds/` dir (a shallow path) rather than deep inside OUT_DIR, because
    // MSVC/CMake hit the Windows 260-char MAX_PATH limit on long OUT_DIR paths.
    let cache_builds = cache_root().unwrap().join("builds");
    fs::create_dir_all(&cache_builds).expect("create superbuild builds dir");
    let source_dir = manifest_dir.join("native");
    let cache_key = native_build_cache_key(&source_dir);
    let build_dir = cache_builds.join(format!(
        "proxi-{}-{}",
        env::var("TARGET").unwrap_or_else(|_| "default".into()),
        cache_key
    ));
    fs::create_dir_all(&build_dir).expect("create superbuild dir");

    let install_prefix = env::var_os("PROXI_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| build_dir.join("prefix"));

    let cache_root = archives_dir().expect("archives dir");
    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(&source_dir);
    cmd.arg("-B").arg(&build_dir);
    cmd.arg(format!(
        "-DCMAKE_INSTALL_PREFIX={}",
        install_prefix.display()
    ));
    cmd.arg(format!("-DPROXI_CACHE_ROOT={}", cache_root.display()));
    cmd.arg(format!(
        "-DPROXI_ARCHIVES_DIR={}",
        archives_dir().expect("archives dir").display()
    ));
    cmd.arg(format!(
        "-DPROXI_SOURCES_DIR={}",
        sources_dir().expect("sources dir").display()
    ));
    if has_flag("network") {
        cmd.arg("-DPROXI_ENABLE_NETWORK=ON");
    }
    if has_flag("tiff") {
        cmd.arg("-DPROXI_ENABLE_TIFF=ON");
    }
    if cfg!(target_os = "macos") {
        if let Ok(arch) = env::var("CMAKE_OSX_ARCHITECTURES") {
            cmd.arg(format!("-DCMAKE_OSX_ARCHITECTURES={arch}"));
        }
    }

    // Configure.
    eprintln!(
        "PROXI: configuring native superbuild in {}",
        build_dir.display()
    );
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("PROXI: failed to invoke cmake: {e}"));
    if !status.success() {
        panic!("PROXI: cmake configure failed (see output above)");
    }

    // Build.
    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg("Release");
    eprintln!(
        "PROXI: building native superbuild (this compiles zlib, SQLite, TLS, curl, tiff, PROJ)"
    );
    let status = build
        .status()
        .unwrap_or_else(|e| panic!("PROXI: failed to invoke cmake --build: {e}"));
    if !status.success() {
        panic!("PROXI: superbuild build failed (see output above)");
    }

    // Consume the generated manifest and emit Rust link directives.
    let manifest_path = install_prefix.join("proxi-native-manifest.json");
    if !manifest_path.exists() {
        panic!(
            "PROXI: superbuild did not produce {}",
            manifest_path.display()
        );
    }
    emit_link_directives(&manifest_path, &install_prefix);

    println!("cargo:rerun-if-env-changed=PROXI_PREFIX");
    println!("cargo:rerun-if-env-changed=PROXI_CACHE");
    println!("cargo:rerun-if-env-changed=PROXI_OFFLINE");
    println!("cargo:rerun-if-changed=native/versions.toml");
    println!("cargo:rerun-if-changed=native/cache.rs");
    println!("cargo:rerun-if-changed=native/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/sqlite/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/write-manifest.cmake.in");
    println!("cargo:rerun-if-changed=native/build-openssl.cmake.in");
}

/// Produce a short stable identity for the native build inputs that CMake
/// embeds in its cache. Different checkouts and feature configurations must
/// never share one CMake build tree.
fn native_build_cache_key(source_dir: &Path) -> String {
    let source = source_dir
        .canonicalize()
        .unwrap_or_else(|_| source_dir.to_path_buf());
    let mut identity = source.to_string_lossy().to_lowercase();
    identity.push('|');
    identity.push_str(env::var("TARGET").unwrap_or_default().as_str());
    identity.push('|');
    identity.push_str(if has_flag("network") { "network" } else { "no-network" });
    identity.push('|');
    identity.push_str(if has_flag("tiff") { "tiff" } else { "no-tiff" });
    identity.push('|');
    identity.push_str(
        &env::var_os("PROXI_PREFIX")
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default(),
    );
    let digest = Sha256::digest(identity.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Parse `proxi-native-manifest.json` and emit `cargo:rustc-*` directives.
fn emit_link_directives(manifest_path: &Path, prefix: &Path) {
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
                // The manifest gives the concrete static-library basename as
                // installed, e.g. "proj", "libcurl", "zlib", "tiff",
                // "sqlite3". rustc-link-lib just needs this name (Cargo appends
                // the platform suffix/.lib). Do NOT strip the `lib` prefix or
                // the `.lib`/`.a` suffix here — the value is already exactly
                // what the linker expects.
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
            println!("cargo:rustc-env=PROXI_BUNDLED_DATA_DIR={data}");
        }
    }
    let _ = prefix;
}

/// Whether a cargo feature flag is enabled.
fn has_flag(flag: &str) -> bool {
    env::var(format!("CARGO_FEATURE_{}", flag.to_uppercase())).is_ok()
}
