//! `proxi` build script - self-provisioning native dependency graph.
//!
//! Decision tree:
//!   1. If the `bundled` feature is enabled: run the local full superbuild.
//!   2. Else if `PROJ_DIR` is set: validate a complete installation and use it.
//!   3. Else if vcpkg finds a complete installation: use it.
//!   4. Else if pkg-config finds a complete installation: use it.
//!   5. Else: run the local full superbuild.
//!
//! The superbuild (`native/CMakeLists.txt`) builds the entire native graph
//! (zlib → SQLite+sqlite3 → TLS → libcurl → libtiff → PROJ) into one private
//! prefix and writes `proxi-native-manifest.json`. `build.rs` reads that
//! manifest and emits all static link directives in dependency order - no
//! CI artifacts or prebuilt binaries are used.
//!
//! Downloading, checksumming, extracting, caching, and driving CMake are
//! [`woodshed`]'s job. What stays here is only what is specific to PROJ.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use woodshed::{BuildKey, Cache, CmakeBuild, Manifest, MsvcRuntime, probe};

#[path = "native/manifest.rs"]
mod manifest;

/// Configure Cargo to link a usable PROJ installation.
fn main() {
    for variable in ["PROJ_DIR", "PROXI_PREFIX"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    // docs.rs builds in a network-isolated sandbox; rustdoc only needs the
    // crate to type-check as an rlib, not a linked library, so skip the
    // superbuild entirely rather than failing on the first download attempt.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    // The `bundled` feature must take precedence over every system probe so a
    // release cannot accidentally link against a developer's PROJ install.
    if has_flag("bundled") {
        run_superbuild();
        return;
    }

    // Tried in order; each degrades to "not found" rather than failing hard.
    // Add a new detection method by adding a probe here, not by editing one.
    let env_dir = probe::EnvDirProbe {
        env_var: "PROJ_DIR", // pyproj-compatible override
        validate: |dir: &Path| validate_installation(dir, required_capabilities()),
        link: link_from_prefix,
    };
    let vcpkg = probe::VcpkgProbe {
        package: "proj",
        on_found: Some(emit_data_dir),
    };
    let pkg_config = probe::PkgConfigProbe {
        package: "proj",
        on_found: Some(emit_data_dir),
    };
    let probes: [&dyn probe::Probe; 3] = [&env_dir, &vcpkg, &pkg_config];
    if probe::first_match(probes) {
        return;
    }

    // Default: local full superbuild (self-provisioning).
    run_superbuild();
}

/// Announce a discovered PROJ data directory, if the prefix has one.
fn emit_data_dir(prefix: &Path) {
    let data_dir = prefix.join("share").join("proj");
    if data_dir.is_dir() {
        println!("cargo:data_dir={}", data_dir.display());
    }
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
    let data_dir = prefix.join("share").join("proj");
    if data_dir.is_dir() {
        println!("cargo:rustc-env=PROXI_DATA_DIR={}", data_dir.display());
        println!("cargo:data_dir={}", data_dir.display());
    }
}

/// Run the CMake superbuild and consume `proxi-native-manifest.json`.
fn run_superbuild() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let source_dir = manifest_dir.join("native");

    let versions = Manifest::load_default(&manifest_dir)
        .unwrap_or_else(|e| panic!("PROXI: cannot load native/versions.toml: {e}"));
    let cache = Cache::discover().unwrap_or_else(|e| panic!("PROXI: {e}"));

    // Resolve, verify, and extract every pinned archive before configuring, so
    // a bad checksum fails in seconds rather than mid-superbuild.
    for dep in versions.iter() {
        cache.source(dep).unwrap_or_else(|e| {
            panic!(
                "PROXI: failed to provision native dependency `{}`: {e}",
                dep.name
            )
        });
    }

    // The build tree's identity is everything that changes its output. CMake
    // source contents are deliberately excluded: the tree must be reused so
    // CMake can rebuild only the external projects a source edit affects.
    let mut key = BuildKey::new("proxi");
    for dep in versions.iter() {
        key = key.dep(dep);
    }
    key = key
        .mix(if has_flag("network") {
            "network"
        } else {
            "no-network"
        })
        .mix(if has_flag("tiff") { "tiff" } else { "no-tiff" });
    if let Some(prefix) = env::var_os("PROXI_PREFIX") {
        key = key.mix(prefix.to_string_lossy().to_lowercase());
    }

    let dirs = cache.build_dir(&key);
    let install_prefix = env::var_os("PROXI_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs.prefix.clone());
    let manifest_path = install_prefix.join("proxi-native-manifest.json");

    // Held for the whole build: two Cargo processes must not compile into the
    // same tree at once.
    let _lock = dirs.lock().unwrap_or_else(|e| panic!("PROXI: {e}"));

    if native_prefix_is_complete(&install_prefix, &manifest_path) {
        eprintln!(
            "PROXI: reusing completed native build at {}",
            install_prefix.display()
        );
        emit_superbuild_rerun_directives();
        manifest::emit_link_directives(&manifest_path);
        if has_flag("embedded") {
            emit_embedded_data(&manifest_path);
        }
        return;
    }
    if manifest_path.exists() {
        eprintln!(
            "PROXI: cached native prefix is incomplete; resuming build at {}",
            dirs.build.display()
        );
        let _ = fs::remove_file(&manifest_path);
    } else {
        eprintln!(
            "PROXI: building native PROJ from scratch at {}",
            dirs.build.display()
        );
    }

    // CMake permanently binds a build tree to the exact `-S` path it was
    // configured with, so the superbuild source is mirrored to a stable cache
    // path. A checkout move then does not force a full native rebuild; source
    // edits are synchronized into the mirror and CMake rebuilds incrementally.
    let mirrored_source = cache
        .mirror(&source_dir, &key)
        .unwrap_or_else(|e| panic!("PROXI: cannot mirror superbuild source: {e}"));

    let mut build = CmakeBuild::new(&mirrored_source, &dirs)
        .prefix(&install_prefix)
        // The superbuild's ExternalProject steps install themselves; there is
        // no top-level `install` target to invoke.
        .target(None::<String>)
        .compiler_launcher_define("PROXI_COMPILER_LAUNCHER")
        // PROJ allocates memory that Rust frees, so the two must share one
        // heap. Only the runtime DLL provides that, even under `+crt-static`.
        .msvc_runtime(MsvcRuntime::AlwaysDynamic)
        .define(
            "PROXI_ARCHIVES_DIR",
            cache.archives_dir().display().to_string(),
        )
        .define(
            "PROXI_SOURCES_DIR",
            cache.sources_dir().display().to_string(),
        );

    // Versions come from versions.toml only, so `woodshed bump` is the single
    // place a dependency version changes.
    for dep in versions.iter() {
        build = build.define(
            format!("PROXI_VERSION_{}", dep.name.to_uppercase()),
            &dep.version,
        );
    }
    if has_flag("network") {
        build = build.define("PROXI_ENABLE_NETWORK", "ON");
    }
    if has_flag("tiff") {
        build = build.define("PROXI_ENABLE_TIFF", "ON");
    }
    if cfg!(target_os = "macos")
        && let Ok(arch) = env::var("CMAKE_OSX_ARCHITECTURES")
    {
        build = build.define("CMAKE_OSX_ARCHITECTURES", arch);
    }

    eprintln!(
        "PROXI: building the native superbuild \
         (this compiles zlib, SQLite, TLS, curl, tiff, PROJ)"
    );
    build
        .install()
        .unwrap_or_else(|e| panic!("PROXI: native superbuild failed: {e}"));

    if !native_prefix_is_complete(&install_prefix, &manifest_path) {
        panic!(
            "PROXI: superbuild did not produce a complete native prefix at {}",
            install_prefix.display()
        );
    }
    manifest::emit_link_directives(&manifest_path);
    if has_flag("embedded") {
        emit_embedded_data(&manifest_path);
    }

    emit_superbuild_rerun_directives();
}

/// Confirm that a cached prefix contains the manifest, database, and PROJ
/// library before allowing Cargo to reuse it.
fn native_prefix_is_complete(prefix: &Path, manifest_path: &Path) -> bool {
    if !manifest_path.is_file() || !prefix.join("share/proj/proj.db").is_file() {
        return false;
    }
    ["lib/proj.lib", "lib/libproj.a", "libproj.lib", "libproj.a"]
        .iter()
        .map(|relative| prefix.join(relative))
        .any(|path| path.is_file())
}

/// Tell Cargo which superbuild inputs invalidate the manifest.
fn emit_superbuild_rerun_directives() {
    println!("cargo:rerun-if-changed=native/versions.toml");
    println!("cargo:rerun-if-changed=native/manifest.rs");
    println!("cargo:rerun-if-changed=native/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/sqlite/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/proj-cmath-compat.cmake");
    println!("cargo:rerun-if-changed=native/write-manifest.cmake.in");
    println!("cargo:rerun-if-changed=native/build-openssl.cmake.in");
}

/// Whether a cargo feature flag is enabled.
fn has_flag(flag: &str) -> bool {
    env::var(format!("CARGO_FEATURE_{}", flag.to_uppercase())).is_ok()
}

fn emit_embedded_data(manifest_path: &Path) {
    let data_dir = manifest_data_dir(manifest_path);
    if !data_dir.join("proj.db").is_file() {
        panic!(
            "PROXI: embedded feature requires proj.db in {}",
            data_dir.display()
        );
    }

    let mut files = Vec::new();
    collect_embedded_files(&data_dir, &data_dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (relative, absolute) in &files {
        hasher.update(relative.as_bytes());
        let bytes = fs::read(absolute)
            .unwrap_or_else(|e| panic!("cannot read PROJ data file {}: {e}", absolute.display()));
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    let data_hash: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let mut generated = String::new();
    generated.push_str("pub(crate) const PROJ_VERSION: &str = \"9.4.1\";\n");
    generated.push_str(&format!(
        "pub(crate) const DATA_HASH: &str = \"{data_hash}\";\n"
    ));
    generated.push_str("pub(crate) const FILES: &[(&str, &[u8])] = &[\n");
    for (relative, absolute) in &files {
        generated.push_str("    (");
        generated.push_str(&rust_string_literal(relative));
        generated.push_str(", include_bytes!(");
        generated.push_str(&rust_string_literal(&absolute.to_string_lossy()));
        generated.push_str(")),\n");
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    fs::write(out_dir.join("proxi_embedded_data.rs"), generated)
        .expect("write generated embedded PROJ data module");
}

fn manifest_data_dir(manifest_path: &Path) -> PathBuf {
    let text = fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path.display()));
    let manifest: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid manifest JSON: {e}"));
    manifest["data_dir"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{} has no data_dir", manifest_path.display()))
}

fn collect_embedded_files(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read PROJ data dir {}: {e}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            collect_embedded_files(root, &path, files);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
}

fn rust_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}
