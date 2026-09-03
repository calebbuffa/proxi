#[cfg(feature = "embedded")]
use crate::errors::ProxiError;
use crate::errors::Result;
use std::path::PathBuf;
#[cfg(feature = "embedded")]
use std::time::Duration;

#[cfg(feature = "embedded")]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/proxi_embedded_data.rs"));
}

#[cfg(not(feature = "embedded"))]
pub(crate) fn materialize() -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(feature = "embedded")]
pub(crate) fn materialize() -> Result<Option<PathBuf>> {
    let data_dir = cache_root()?.join("share").join("proj");
    let marker = data_dir.join(".proxi-data-hash");
    if data_dir.join("proj.db").is_file()
        && std::fs::read_to_string(&marker).ok().as_deref() == Some(generated::DATA_HASH)
    {
        return Ok(Some(data_dir));
    }

    let parent = data_dir
        .parent()
        .ok_or_else(|| ProxiError::ContextConfiguration {
            message: format!(
                "invalid embedded PROJ data directory {}",
                data_dir.display()
            ),
        })?;
    std::fs::create_dir_all(parent).map_err(|error| ProxiError::ContextConfiguration {
        message: format!(
            "create embedded PROJ data cache {}: {error}",
            parent.display()
        ),
    })?;
    let _lock = acquire_lock(&parent.join(".proxi-data.lock"))?;
    // Another process may have installed the data while we waited.
    if data_dir.join("proj.db").is_file()
        && std::fs::read_to_string(&marker).ok().as_deref() == Some(generated::DATA_HASH)
    {
        return Ok(Some(data_dir));
    }

    let temp_dir = parent.join(format!(
        ".extract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).map_err(|error| ProxiError::ContextConfiguration {
        message: format!(
            "create embedded PROJ data temp dir {}: {error}",
            temp_dir.display()
        ),
    })?;

    for &(relative, bytes) in generated::FILES {
        let path = temp_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ProxiError::ContextConfiguration {
                message: format!(
                    "create embedded PROJ data dir {}: {error}",
                    parent.display()
                ),
            })?;
        }
        std::fs::write(&path, bytes).map_err(|error| ProxiError::ContextConfiguration {
            message: format!("write embedded PROJ data file {}: {error}", path.display()),
        })?;
    }
    std::fs::write(temp_dir.join(".proxi-data-hash"), generated::DATA_HASH).map_err(|error| {
        ProxiError::ContextConfiguration {
            message: format!("write embedded PROJ data marker: {error}"),
        }
    })?;

    if data_dir.exists() {
        std::fs::remove_dir_all(&data_dir).map_err(|error| ProxiError::ContextConfiguration {
            message: format!(
                "replace embedded PROJ data cache {}: {error}",
                data_dir.display()
            ),
        })?;
    }
    match std::fs::rename(&temp_dir, &data_dir) {
        Ok(()) => Ok(Some(data_dir)),
        Err(error) if data_dir.join("proj.db").is_file() => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            let _ = error;
            Ok(Some(data_dir))
        }
        Err(error) => Err(ProxiError::ContextConfiguration {
            message: format!(
                "install embedded PROJ data cache {}: {error}",
                data_dir.display()
            ),
        }),
    }
}

#[cfg(feature = "embedded")]
struct InstallLock {
    path: PathBuf,
}

#[cfg(feature = "embedded")]
impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(feature = "embedded")]
fn acquire_lock(path: &std::path::Path) -> Result<InstallLock> {
    for _ in 0..600 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => {
                return Ok(InstallLock {
                    path: path.to_path_buf(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(ProxiError::ContextConfiguration {
                    message: format!("create embedded PROJ data lock {}: {error}", path.display()),
                });
            }
        }
    }
    Err(ProxiError::ContextConfiguration {
        message: format!(
            "timed out waiting for embedded PROJ data lock {}",
            path.display()
        ),
    })
}

#[cfg(feature = "embedded")]
fn cache_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PROXI_EMBEDDED_DATA_CACHE") {
        return Ok(PathBuf::from(path)
            .join("proj")
            .join(generated::PROJ_VERSION)
            .join(generated::DATA_HASH));
    }

    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("TEMP").map(PathBuf::from))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    };

    base.map(|base| {
        base.join("proxi")
            .join("proj")
            .join(generated::PROJ_VERSION)
            .join(generated::DATA_HASH)
    })
    .ok_or_else(|| ProxiError::ContextConfiguration {
        message: "cannot determine a writable cache directory for embedded PROJ data".to_string(),
    })
}
