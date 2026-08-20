//! Generic native-build toolchain selection: pick the fastest CMake generator
//! and compiler cache available, always with a fallback that every machine
//! has. Nothing here is specific to any particular C/C++ dependency, so
//! extending it (e.g. adding a linker preference) never touches call sites.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Best-available toolchain for driving a CMake superbuild, detected once and
/// reused for both the configure and build steps.
pub struct ToolchainChoice {
    generator: Option<String>,
    compiler_launcher: Option<PathBuf>,
    jobs: usize,
}

impl ToolchainChoice {
    /// Detect the fastest safe toolchain for the current target.
    pub fn detect() -> Self {
        // Ninja needs cl.exe's env (vcvarsall) on MSVC; the VS generator
        // doesn't, so only force Ninja there if that env is already set up.
        let is_msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
        let msvc_env_ready = env::var_os("INCLUDE").is_some() && env::var_os("LIB").is_some();
        let ninja = find_on_path("ninja").filter(|_| !is_msvc || msvc_env_ready);

        // ccache's MSVC (cl.exe) support is unreliable; sccache supports it natively.
        let compiler_launcher = find_on_path("sccache")
            .or_else(|| (!cfg!(windows)).then(|| find_on_path("ccache")).flatten());

        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        Self {
            generator: ninja.map(|_| "Ninja".to_string()),
            compiler_launcher,
            jobs,
        }
    }

    /// Recreate `build_dir` if it was configured against a different
    /// generator or a different source path than this build would use — both
    /// are hard CMake invariants that can't be reconciled by reconfiguring in
    /// place, so a stale match means starting the build tree over.
    pub fn reset_build_dir_if_stale(&self, build_dir: &Path, source_dir: &Path) {
        let Ok(existing) = fs::read_to_string(build_dir.join("CMakeCache.txt")) else {
            return;
        };
        let existing_gen = existing
            .lines()
            .find_map(|l| l.strip_prefix("CMAKE_GENERATOR:INTERNAL="))
            .unwrap_or("");
        let desired = self.generator.as_deref().unwrap_or("");
        let generator_changed = if desired.is_empty() {
            existing_gen == "Ninja"
        } else {
            existing_gen != desired
        };

        // CMake binds a build tree to the exact source path it was configured
        // with; reusing the tree for the same content at a different path
        // (e.g. `cargo publish`'s verification build) errors out otherwise.
        let existing_home = existing
            .lines()
            .find_map(|l| l.strip_prefix("CMAKE_HOME_DIRECTORY:INTERNAL="))
            .unwrap_or("")
            .to_lowercase();
        let desired_home = source_dir
            .canonicalize()
            .unwrap_or_else(|_| source_dir.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let source_moved = !existing_home.is_empty() && existing_home != desired_home;

        if generator_changed || source_moved {
            eprintln!(
                "PROXI: build tree configuration changed; recreating {}",
                build_dir.display()
            );
            force_recreate_dir(build_dir);
        }
    }

    /// Append this choice's flags to a `cmake -S -B ...` configure command.
    /// `compiler_launcher_define` is the `-D<NAME>=...` cache var the
    /// consuming CMakeLists.txt reads (caller-supplied since the var name is
    /// project-specific).
    pub fn apply_to_configure(&self, cmd: &mut Command, compiler_launcher_define: &str) {
        if let Some(g) = &self.generator {
            eprintln!("PROXI: using {g} generator");
            cmd.arg("-G").arg(g);
        }
        if let Some(launcher) = &self.compiler_launcher {
            eprintln!("PROXI: using compiler cache {}", launcher.display());
            cmd.arg(format!(
                "-D{compiler_launcher_define}={}",
                launcher.display()
            ));
        }
    }

    /// Append `--parallel` to a `cmake --build ...` command and set the env
    /// var that nested `ExternalProject_Add` build steps inherit. Returns the
    /// job count so the caller can log it.
    pub fn apply_to_build(&self, cmd: &mut Command) -> usize {
        cmd.arg("--parallel").arg(self.jobs.to_string());
        cmd.env("CMAKE_BUILD_PARALLEL_LEVEL", self.jobs.to_string());
        self.jobs
    }
}

/// Locate an executable on `PATH`, trying the Windows `.exe` suffix too.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var).find_map(|dir| {
        let plain = dir.join(name);
        if plain.is_file() {
            return Some(plain);
        }
        let exe = dir.join(format!("{name}.exe"));
        exe.is_file().then_some(exe)
    })
}

/// Delete and recreate `dir`, tolerating transient Windows file locks (a
/// just-exited cmake/ninja/msbuild process, or an antivirus scan, can hold a
/// file inside briefly). Retries the delete, then falls back to renaming the
/// tree aside — a rename doesn't require handles inside it to be closed —
/// before giving up with an actionable error.
fn force_recreate_dir(dir: &Path) {
    let mut last_err = None;
    for attempt in 0..10u32 {
        match fs::remove_dir_all(dir) {
            Ok(()) => {
                fs::create_dir_all(dir).expect("recreate superbuild dir");
                return;
            }
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(200 * (attempt as u64 + 1)));
            }
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let stale = PathBuf::from(format!("{}-stale-{stamp}", dir.display()));
    if fs::rename(dir, &stale).is_ok() {
        // Best-effort cleanup; a leftover `-stale-*` dir only wastes disk space.
        let _ = fs::remove_dir_all(&stale);
        fs::create_dir_all(dir).expect("recreate superbuild dir");
        return;
    }

    panic!(
        "PROXI: could not recreate build tree {} ({}).\n\
         \x20 another process (cmake, ninja, msbuild, an antivirus scan) may still \
         have a file open inside it — close it and rerun, or delete the directory manually.",
        dir.display(),
        last_err.map(|e| e.to_string()).unwrap_or_default()
    );
}
