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

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime};

#[path = "native/cache.rs"]
mod cache;
#[path = "native/manifest.rs"]
mod manifest;
#[path = "native/process.rs"]
mod process;
#[path = "native/system_probe.rs"]
mod system_probe;
#[path = "native/toolchain.rs"]
mod toolchain;

use cache::{NativeVersions, archives_dir, cache_root, sources_dir};
use system_probe::{EnvDirProbe, PkgConfigProbe, SystemProbe, VcpkgProbe};
use toolchain::ToolchainChoice;

/// Configure Cargo to link a usable PROJ installation.
fn main() {
    for variable in ["PROXI_BUNDLED", "PROJ_DIR", "VCPKG_ROOT", "PKG_CONFIG_PATH"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    // docs.rs builds in a network-isolated sandbox; rustdoc only needs the
    // crate to type-check as an rlib, not a linked library, so skip the
    // superbuild entirely rather than failing on the first download attempt.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    // Respect explicit "force local superbuild" and the Cargo `bundled`
    // feature. The feature must take precedence over every system probe so a
    // release cannot accidentally link against a developer's PROJ install.
    if env::var("PROXI_BUNDLED").map(|v| v == "1").unwrap_or(false) || has_flag("bundled") {
        run_superbuild();
        return;
    }

    // Tried in order; each degrades to "not found" rather than failing hard.
    // Add a new detection method by adding a probe here, not by editing one.
    let probes: Vec<Box<dyn SystemProbe>> = vec![
        Box::new(EnvDirProbe {
            env_var: "PROJ_DIR", // pyproj-compatible override
            validate: |dir: &Path| validate_installation(dir, required_capabilities()),
            link: link_from_prefix,
        }),
        Box::new(VcpkgProbe { package: "proj" }),
        Box::new(PkgConfigProbe {
            package: "proj",
            skip: has_flag("bundled"),
        }),
    ];
    for probe in &probes {
        if probe.try_use() {
            return;
        }
    }

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
    let data_dir = prefix.join("share").join("proj");
    if data_dir.is_dir() {
        println!("cargo:data_dir={}", data_dir.display());
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
        cache::ensure_archive(dep)
            .unwrap_or_else(|e| panic!("PROXI: failed to resolve native dependency `{name}`: {e}"));
        cache::ensure_source(dep)
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

    // CMake permanently binds a build tree to the exact `-S` path it was
    // configured with. The checkout path varies (a different clone, a
    // `cargo publish` verification copy, a new crate version's registry
    // dir) even when `native/`'s content doesn't, which would otherwise force
    // a wipe-and-recompile every time. Mirroring into a path keyed by the
    // same content hash gives CMake a stable `-S` target so identical
    // content always reuses the same build tree, regardless of where the
    // original checkout lives.
    let mirrored_source_dir = cache_root()
        .unwrap()
        .join("superbuild-src")
        .join(&cache_key);
    mirror_source_dir(&source_dir, &mirrored_source_dir);

    let install_prefix = env::var_os("PROXI_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| build_dir.join("prefix"));
    let manifest_path = install_prefix.join("proxi-native-manifest.json");
    let _cache_lock = acquire_native_build_lock(&cache_builds, &build_dir);

    if native_prefix_is_complete(&install_prefix, &manifest_path) {
        eprintln!(
            "PROXI: reusing completed native build at {}",
            install_prefix.display()
        );
        emit_superbuild_rerun_directives();
        manifest::emit_link_directives(&manifest_path);
        return;
    }
    if manifest_path.exists() {
        eprintln!(
            "PROXI: cached native prefix is incomplete; resuming build at {}",
            build_dir.display()
        );
        let _ = fs::remove_file(&manifest_path);
    } else {
        eprintln!(
            "PROXI: building native PROJ from scratch at {}",
            build_dir.display()
        );
    }

    // Best-available toolchain, always with a fallback that every machine has.
    let toolchain = ToolchainChoice::detect();
    toolchain.reset_build_dir_if_stale(&build_dir, &mirrored_source_dir);

    let cache_root_dir = archives_dir().expect("archives dir");
    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(&mirrored_source_dir);
    cmd.arg("-B").arg(&build_dir);
    toolchain.apply_to_configure(&mut cmd, "PROXI_COMPILER_LAUNCHER");
    cmd.arg(format!(
        "-DCMAKE_INSTALL_PREFIX={}",
        install_prefix.display()
    ));
    cmd.arg(format!("-DPROXI_CACHE_ROOT={}", cache_root_dir.display()));
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
    let status = process::run(&mut cmd, "cmake configure")
        .unwrap_or_else(|e| panic!("PROXI: failed to invoke cmake: {e}"));
    if !status.success() {
        panic!("PROXI: cmake configure failed (see output above)");
    }

    // Build. `--parallel` covers this invocation; the env var propagates to
    // the nested `cmake --build` processes each ExternalProject step spawns.
    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg("Release");
    let jobs = toolchain.apply_to_build(&mut build);
    eprintln!(
        "PROXI: building native superbuild with {jobs} parallel job(s) \
         (this compiles zlib, SQLite, TLS, curl, tiff, PROJ)"
    );
    let status = process::run(&mut build, "cmake build")
        .unwrap_or_else(|e| panic!("PROXI: failed to invoke cmake --build: {e}"));
    if !status.success() {
        panic!("PROXI: superbuild build failed (see output above)");
    }

    // Consume the generated manifest and emit Rust link directives.
    if !native_prefix_is_complete(&install_prefix, &manifest_path) {
        panic!(
            "PROXI: superbuild did not produce a complete native prefix at {}",
            install_prefix.display()
        );
    }
    manifest::emit_link_directives(&manifest_path);

    emit_superbuild_rerun_directives();
}

/// Prevent concurrent Cargo processes from building the same native cache key.
/// A lock is reclaimed as soon as its recorded owner process is gone. The age
/// check is only a fallback for malformed or legacy lock files without a PID.
fn acquire_native_build_lock(cache_builds: &Path, build_dir: &Path) -> NativeBuildLock {
    let lock_path = cache_builds.join(format!(
        "{}.lock",
        build_dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => {
                let _ = serde_json::to_writer(
                    file,
                    &serde_json::json!({
                        "pid": std::process::id(),
                        "started": format!("{:?}", SystemTime::now()),
                    }),
                );
                return NativeBuildLock { path: lock_path };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let lock_owner = fs::read_to_string(&lock_path)
                    .ok()
                    .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
                    .and_then(|value| value.get("pid").and_then(|pid| pid.as_u64()))
                    .map(|pid| pid as u32);
                let owner_is_dead = lock_owner.is_some_and(|pid| !process::is_alive(pid));
                let legacy_lock_is_stale = lock_owner.is_none() && fs::metadata(&lock_path)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|time| SystemTime::now().duration_since(time).ok())
                    .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
                if owner_is_dead || legacy_lock_is_stale {
                    eprintln!(
                        "PROXI: reclaiming abandoned native build lock {}",
                        lock_path.display()
                    );
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                eprintln!(
                    "PROXI: waiting for another build of {}",
                    build_dir.display()
                );
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => panic!(
                "PROXI: cannot create native build lock {}: {error}",
                lock_path.display()
            ),
        }
    }
}

struct NativeBuildLock {
    path: PathBuf,
}

impl Drop for NativeBuildLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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

/// Tell Cargo which build-script and superbuild inputs invalidate the manifest.
fn emit_superbuild_rerun_directives() {
    println!("cargo:rerun-if-env-changed=PROXI_PREFIX");
    println!("cargo:rerun-if-env-changed=PROXI_CACHE");
    println!("cargo:rerun-if-env-changed=PROXI_OFFLINE");
    println!("cargo:rerun-if-changed=native/versions.toml");
    println!("cargo:rerun-if-changed=native/cache.rs");
    println!("cargo:rerun-if-changed=native/toolchain.rs");
    println!("cargo:rerun-if-changed=native/system_probe.rs");
    println!("cargo:rerun-if-changed=native/process.rs");
    println!("cargo:rerun-if-changed=native/manifest.rs");
    println!("cargo:rerun-if-changed=native/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/sqlite/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/proj-cmath-compat.cmake");
    println!("cargo:rerun-if-changed=native/write-manifest.cmake.in");
    println!("cargo:rerun-if-changed=native/build-openssl.cmake.in");
}

/// Produce a short stable identity for the native build inputs that CMake
/// embeds in its cache. Hashed by *content*, not by checkout path, so
/// installing the same version from a different folder (a fresh `git clone`,
/// a new `cargo install --path`, etc.) reuses the existing compiled build
/// tree instead of recompiling the whole native graph from scratch.
fn native_build_cache_key(source_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    hash_dir_contents(source_dir, source_dir, &mut hasher);
    hasher.update(env::var("TARGET").unwrap_or_default().as_bytes());
    hasher.update(if has_flag("network") {
        b"network".as_slice()
    } else {
        b"no-network".as_slice()
    });
    hasher.update(if has_flag("tiff") {
        b"tiff".as_slice()
    } else {
        b"no-tiff".as_slice()
    });
    if let Some(prefix) = env::var_os("PROXI_PREFIX") {
        hasher.update(prefix.to_string_lossy().to_lowercase().as_bytes());
    }
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Fold every file under `dir` (path relative to `root` + contents) into
/// `hasher`, in a stable order, so identical `native/` trees hash identically
/// regardless of where on disk they happen to live.
fn hash_dir_contents(root: &Path, dir: &Path, hasher: &mut Sha256) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            hash_dir_contents(root, &path, hasher);
        } else if let Ok(bytes) = fs::read(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            hasher.update(rel.to_string_lossy().to_lowercase().as_bytes());
            hasher.update(&bytes);
        }
    }
}

/// Whether a cargo feature flag is enabled.
fn has_flag(flag: &str) -> bool {
    env::var(format!("CARGO_FEATURE_{}", flag.to_uppercase())).is_ok()
}

/// Copy `src` into `dest` if `dest` doesn't already exist. `dest` is keyed by
/// a content hash of `src`, so an existing `dest` is guaranteed (short of a
/// hash collision) to already hold identical content — never re-copied.
fn mirror_source_dir(src: &Path, dest: &Path) {
    if dest.exists() {
        return;
    }
    fs::create_dir_all(dest).expect("create mirrored superbuild source dir");
    copy_dir_recursive(src, dest);
}

/// Recursively copy every entry under `src` into `dest`.
fn copy_dir_recursive(src: &Path, dest: &Path) {
    for entry in fs::read_dir(src)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()))
        .flatten()
    {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target)
                .unwrap_or_else(|e| panic!("cannot create {}: {e}", target.display()));
            copy_dir_recursive(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap_or_else(|e| {
                panic!(
                    "cannot mirror {} to {}: {e}",
                    path.display(),
                    target.display()
                )
            });
        }
    }
}
