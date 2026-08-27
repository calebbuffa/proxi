//! Strategy pattern for "is there already a usable native install?". Add a
//! new detection method by implementing `SystemProbe` and pushing it into the
//! chain in `build.rs` — existing probes and the chain itself never need to
//! change (open for extension, closed for modification).

use std::env;
use std::path::{Path, PathBuf};

/// A single way to discover and use an existing native installation. On
/// success, a probe emits its own `cargo:` directives before returning `true`.
pub trait SystemProbe {
    fn try_use(&self) -> bool;
}

/// An env var (e.g. `PROJ_DIR`) pointing at a prefix to validate and link.
pub struct EnvDirProbe<V, L>
where
    V: Fn(&Path) -> Result<(), String>,
    L: Fn(&Path),
{
    pub env_var: &'static str,
    pub validate: V,
    pub link: L,
}

impl<V, L> SystemProbe for EnvDirProbe<V, L>
where
    V: Fn(&Path) -> Result<(), String>,
    L: Fn(&Path),
{
    fn try_use(&self) -> bool {
        let Some(dir) = env::var_os(self.env_var) else {
            return false;
        };
        let dir = PathBuf::from(dir);
        match (self.validate)(&dir) {
            Ok(()) => {
                (self.link)(&dir);
                println!("cargo:rerun-if-env-changed={}", self.env_var);
                true
            }
            Err(e) => {
                eprintln!(
                    "PROXI: {}={} is set but does not satisfy requested capabilities: {e}\n\
                     \x20 falling back to the next detection method.",
                    self.env_var,
                    dir.display()
                );
                false
            }
        }
    }
}

/// vcpkg (Windows/MSVC) discovery for a single package.
pub struct VcpkgProbe {
    pub package: &'static str,
}

impl SystemProbe for VcpkgProbe {
    #[cfg(target_env = "msvc")]
    fn try_use(&self) -> bool {
        let mut config = vcpkg::Config::new();
        if let Ok(lib) = config.find_package(self.package) {
            if let Some(inc) = lib.include_paths.first() {
                eprintln!("PROXI: using vcpkg {} at {}", self.package, inc.display());
                let prefix = inc.parent().unwrap_or(inc);
                let data_dir = prefix.join("share").join("proj");
                if data_dir.is_dir() {
                    println!("cargo:data_dir={}", data_dir.display());
                }
                println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
                return true;
            }
        }
        false
    }

    #[cfg(not(target_env = "msvc"))]
    fn try_use(&self) -> bool {
        false
    }
}

/// pkg-config (Unix) discovery for a single package, skippable via a flag
/// (e.g. the crate's own `bundled` feature forcing a from-source build).
// Fields are only read by the `#[cfg(unix)]` impl below; unused elsewhere.
#[allow(dead_code)]
pub struct PkgConfigProbe {
    pub package: &'static str,
    pub skip: bool,
}

impl SystemProbe for PkgConfigProbe {
    #[cfg(unix)]
    fn try_use(&self) -> bool {
        if self.skip {
            return false;
        }
        if let Ok(pkg) = pkg_config::Config::new().probe(self.package) {
            eprintln!(
                "PROXI: using system {} via pkg-config: {}",
                self.package, pkg.version
            );
            if let Some(lib_dir) = pkg.link_paths.first() {
                if let Some(prefix) = lib_dir.parent() {
                    let data_dir = prefix.join("share").join("proj");
                    if data_dir.is_dir() {
                        println!("cargo:data_dir={}", data_dir.display());
                    }
                }
            }
            println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
            return true;
        }
        false
    }

    #[cfg(not(unix))]
    fn try_use(&self) -> bool {
        false
    }
}
