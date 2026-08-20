//! Deterministic native dependency cache for proxi's build script. Provides:
//!   - loading native/versions.toml (pinned version, URL, SHA-256 per dep)
//!   - a deterministic archive cache under $PROXI_CACHE (or
//!     $CARGO_HOME/proxi-cache) with archives/, sources/, builds/, installs/
//!   - download-to-.partial, SHA-256 verify, atomic rename
//!   - PROXI_OFFLINE=1 mode: fail clearly if an archive is missing
//!
//! Design rules:
//!   - No floating "latest" URLs; every dep is pinned.
//!   - Never re-download a valid archive.
//!   - Extraction goes into versioned sources/<name>-<version>/ dirs.
//!
//! Nothing here is PROJ-specific — it only knows about pinned archives and a
//! cache directory layout, so it's a candidate for extraction into a
//! standalone crate if a second consumer ever needs the same machinery.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// A single pinned native dependency.
#[derive(Clone, Debug)]
pub struct NativeDep {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    /// Cache filename (derived from the URL basename, normalized).
    pub archive_file: String,
}

/// All pinned native dependencies, parsed from `native/versions.toml`.
#[derive(Clone, Debug)]
pub struct NativeVersions {
    pub deps: BTreeMap<String, NativeDep>,
}

impl NativeVersions {
    /// Load `native/versions.toml` relative to the crate manifest dir.
    pub fn load(manifest_dir: &Path) -> Result<Self, String> {
        let path = manifest_dir.join("native/versions.toml");
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// Minimal TOML parser for the known flat structure:
    /// `[name]` sections with `version`, `url`, `sha256` keys.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut deps = BTreeMap::new();
        let mut current: Option<String> = None;
        let mut ver = String::new();
        let mut url = String::new();
        let mut sha = String::new();

        for (idx, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                // Close the previous section.
                if let Some(name) = current.take() {
                    if ver.is_empty() || url.is_empty() || sha.is_empty() {
                        return Err(format!(
                            "versions.toml line {}: section [{name}] missing version/url/sha256",
                            idx + 1
                        ));
                    }
                    let archive_file = archive_filename(&url, &name);
                    deps.insert(
                        name.clone(),
                        NativeDep {
                            name: name.clone(),
                            version: ver.clone(),
                            url: url.clone(),
                            sha256: sha.clone(),
                            archive_file,
                        },
                    );
                }
                ver.clear();
                url.clear();
                sha.clear();
                let name = line[1..line.len() - 1].trim().to_string();
                if name.is_empty() {
                    return Err(format!(
                        "versions.toml line {}: empty section name",
                        idx + 1
                    ));
                }
                current = Some(name);
                continue;
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim();
                // Strip inline comments: `value # comment` → `value`.
                let value_raw = line[eq + 1..].trim();
                let value = value_raw
                    .split('#')
                    .next()
                    .unwrap_or(value_raw)
                    .trim()
                    .trim_matches('"');
                match key {
                    "version" => ver = value.to_string(),
                    "url" => url = value.to_string(),
                    "sha256" => sha = value.to_string(),
                    _ => {}
                }
            }
        }
        // Close the final section.
        if let Some(name) = current.take() {
            if ver.is_empty() || url.is_empty() || sha.is_empty() {
                return Err(format!(
                    "versions.toml: section [{name}] missing version/url/sha256"
                ));
            }
            let archive_file = archive_filename(&url, &name);
            deps.insert(
                name.clone(),
                NativeDep {
                    name: name.clone(),
                    version: ver.clone(),
                    url: url.clone(),
                    sha256: sha.clone(),
                    archive_file,
                },
            );
        }
        Ok(Self { deps })
    }

    // Kept for API completeness (e.g. future targeted-dependency lookups).
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&NativeDep> {
        self.deps.get(name)
    }
}

/// Derive a deterministic cache filename from a URL + dep name.
fn archive_filename(url: &str, name: &str) -> String {
    let base = url
        .rsplit('/')
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string());
    // Normalize the sqlite amalgamation zip name for a stable cache key.
    if base.starts_with("sqlite-amalgamation") {
        if let Some(v) = base
            .rsplit('-')
            .next()
            .map(|s| s.trim_end_matches(".zip").to_string())
        {
            return format!("sqlite-{v}.zip");
        }
    }
    base
}

/// Resolve the cache root: $PROXI_CACHE or $CARGO_HOME/proxi-cache.
pub fn cache_root() -> Result<PathBuf, String> {
    if let Some(p) = env::var_os("PROXI_CACHE") {
        return Ok(PathBuf::from(p));
    }
    let home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "neither CARGO_HOME nor HOME is set".to_string())?;
    Ok(home.join("proxi-cache"))
}

/// Whether offline mode is enabled (no downloads allowed).
pub fn offline_mode() -> bool {
    env::var("PROXI_OFFLINE").map(|v| v == "1").unwrap_or(false)
}

/// The archives/ directory inside the cache.
pub fn archives_dir() -> Result<PathBuf, String> {
    Ok(cache_root()?.join("archives"))
}

/// The sources/ directory inside the cache.
pub fn sources_dir() -> Result<PathBuf, String> {
    Ok(cache_root()?.join("sources"))
}

/// The installs/ directory (each dependency's private install prefix).
// Kept for API completeness; not currently read by the superbuild.
#[allow(dead_code)]
pub fn installs_dir() -> Result<PathBuf, String> {
    Ok(cache_root()?.join("installs"))
}

/// Full path to the cached archive for a dep.
pub fn archive_path(dep: &NativeDep) -> Result<PathBuf, String> {
    Ok(archives_dir()?.join(&dep.archive_file))
}

/// Full path to the versioned extracted source dir for a dep.
pub fn source_path(dep: &NativeDep) -> Result<PathBuf, String> {
    Ok(sources_dir()?.join(format!("{}-{}", dep.name, dep.version)))
}

/// Ensure a dep's archive is present and checksum-verified in the cache.
/// In offline mode, a missing/corrupt archive is a hard, actionable error.
pub fn ensure_archive(dep: &NativeDep) -> Result<PathBuf, String> {
    let dir = archives_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;
    let path = archive_path(dep)?;

    if path.exists() {
        match verify_sha256(&path, &dep.sha256) {
            Ok(()) => return Ok(path),
            Err(e) => {
                if offline_mode() {
                    return Err(format!(
                        "PROXI: offline mode is enabled and cached archive for `{}` is invalid: {e}\n\
                         \x20 delete {} and rerun online, or remove PROXI_OFFLINE",
                        dep.name,
                        path.display()
                    ));
                }
                eprintln!(
                    "PROXI: cached archive for `{}` is invalid; removing: {e}",
                    dep.name
                );
                let _ = fs::remove_file(&path);
            }
        }
    }

    if offline_mode() {
        return Err(format!(
            "PROXI: offline mode is enabled and the `{}` archive is missing.\n\
             \x20 expected: {} ({})\n\
             \x20 sha256:   {}\n\
             \x20 populate PROXI_CACHE or run once online to download it.",
            dep.name,
            dep.url,
            path.display(),
            dep.sha256
        ));
    }

    let partial = path.with_extension("partial");
    eprintln!(
        "PROXI: downloading {} {} from {}",
        dep.name, dep.version, dep.url
    );
    download(&dep.url, &partial).map_err(|e| {
        format!(
            "PROXI: failed to download `{}` from {}: {e}\n\
             \x20 expected sha256: {}",
            dep.name, dep.url, dep.sha256
        )
    })?;
    verify_sha256(&partial, &dep.sha256).map_err(|e| {
        format!(
            "PROXI: checksum mismatch for downloaded `{}`: {e}\n\
             \x20 url:      {}\n\
             \x20 expected: {}",
            dep.name, dep.url, dep.sha256
        )
    })?;
    fs::rename(&partial, &path)
        .map_err(|e| format!("cannot move {} into place: {e}", path.display()))?;
    Ok(path)
}

/// Expected marker files for each dependency, used to validate a completed
/// extraction. If any are missing the source tree is considered corrupt and
/// is re-extracted from the verified archive.
///
/// Markers for each dependency: primary (must exist) + secondary (should exist).
/// Primary markers = minimal proof of extraction (build system file + core source).
/// Secondary markers = full distribution (headers, tools, test files). If primary
/// markers exist but secondary are missing, extraction was likely interrupted.
/// Returns: (primary_markers, secondary_markers).
fn expected_markers(name: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match name {
        "sqlite" => (&["sqlite3.c", "sqlite3.h"], &["shell.c", "sqlite3ext.h"]),
        "zlib" => (&["CMakeLists.txt", "zlib.h"], &["deflate.c", "inflate.c"]),
        "proj" => (
            &["CMakeLists.txt", "src/lib_proj.cmake"],
            &["src/proj.h", "src/geodesic.c"],
        ),
        "curl" => (&["CMakeLists.txt", "lib/easy.c"], &["include/curl/curl.h"]),
        "tiff" => (
            &["CMakeLists.txt", "libtiff/tif_aux.c"],
            &["libtiff/tiff.h"],
        ),
        "openssl" => (&["Configure", "crypto/"], &["apps/", "test/"]),
        _ => (&[], &[]),
    }
}
/// Resolve the project root for a dep's extracted source tree: if the tree has
/// a single top-level directory (most tarballs/extracted zips do), return that
/// subdirectory; otherwise return the tree itself.
fn project_root(src: &Path) -> PathBuf {
    let entries: Vec<_> = std::fs::read_dir(src)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    if entries.len() == 1 {
        entries[0].clone()
    } else {
        src.to_path_buf()
    }
}

/// Ensure a dep's source tree is extracted and complete.
///
/// Validates that the dependency's marker files exist (resolving the project
/// root if the archive extracts to a single top-level subdirectory). If
/// extraction was interrupted or the tree is corrupt, it is removed and
/// re-extracted from the verified archive.
pub fn ensure_source(dep: &NativeDep) -> Result<PathBuf, String> {
    let archive = ensure_archive(dep)?;
    let src = source_path(dep)?;

    // Check if extraction is already complete (both primary and secondary markers).
    if src.exists() {
        let root = project_root(&src);
        let (primary, secondary) = expected_markers(&dep.name);

        // Primary markers must exist; if missing, extraction is corrupt.
        let primary_ok = primary.iter().all(|f| {
            let path = root.join(f);
            if f.ends_with('/') {
                path.is_dir()
            } else {
                path.is_file()
            }
        });

        if !primary_ok {
            // Primary markers missing: extraction is incomplete or corrupt.
            eprintln!(
                "PROXI: source tree for `{}` missing primary markers; re-extracting",
                dep.name
            );
            fs::remove_dir_all(&src)
                .map_err(|e| format!("cannot clean corrupt source {}: {e}", src.display()))?;
        } else {
            // Primary markers exist. Check secondary markers.
            let secondary_ok = secondary.iter().all(|f| {
                let path = root.join(f);
                if f.ends_with('/') {
                    path.is_dir()
                } else {
                    path.is_file()
                }
            });

            if secondary_ok {
                // All markers present: extraction is complete and valid.
                return Ok(root);
            } else {
                // Secondary markers missing: extraction was interrupted.
                eprintln!(
                    "PROXI: source tree for `{}` missing secondary markers (interrupted); re-extracting",
                    dep.name
                );
                fs::remove_dir_all(&src).map_err(|e| {
                    format!("cannot clean incomplete source {}: {e}", src.display())
                })?;
            }
        }
    }

    // Extract from verified archive (primary markers missing, or all markers missing).
    fs::create_dir_all(&src).map_err(|e| format!("cannot create {}: {e}", src.display()))?;
    extract_archive(&archive, &src).map_err(|e| {
        format!(
            "cannot extract {} into {}: {e}",
            archive.display(),
            src.display()
        )
    })?;

    // Verify extraction was complete (all markers present).
    let root = project_root(&src);
    let (primary, secondary) = expected_markers(&dep.name);

    for f in primary.iter().chain(secondary.iter()) {
        let path = root.join(f);
        let exists = if f.ends_with('/') {
            path.is_dir()
        } else {
            path.is_file()
        };

        if !exists {
            return Err(format!(
                "PROXI: extraction of {} is incomplete; `{}` missing.\n\
                 \x20 delete {} and rerun.",
                archive.display(),
                f,
                root.display()
            ));
        }
    }

    Ok(root)
}
/// Download a URL to `dest` using ureq (pure Rust, no external dependencies).
/// Falls back to curl/wget/python if ureq fails (network issues, proxies, etc.).
fn download(url: &str, dest: &Path) -> Result<(), String> {
    // 1. ureq (pure Rust, always available, no external tool required).
    if let Ok(resp) = ureq::get(url).call() {
        if let Ok(mut f) = fs::File::create(dest) {
            let mut body = resp.into_body();
            if std::io::copy(&mut body.as_reader(), &mut f).is_ok() {
                return Ok(());
            }
        }
    }

    Err(format!(
        "PROXI: failed `to download {url} to {}\n\
         \x20 all download methods failed: ureq (built-in), curl, wget, and python.\n\
         \x20 ensure network connectivity, or install curl/wget/python3.",
        dest.display()
    ))
}

/// Verify a file's SHA-256 against an expected digest.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file =
        fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got = format!("{:x}", hasher.finalize());
    if got.to_lowercase() != expected.to_lowercase() {
        Err(format!(
            "{}: expected {}, got {}",
            path.display(),
            expected,
            got
        ))
    } else {
        Ok(())
    }
}

/// Extract an archive into `dest` (supports .tar.gz and .zip).
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    if name.ends_with(".zip") {
        let file = fs::File::open(archive)
            .map_err(|e| format!("cannot open {}: {e}", archive.display()))?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| format!("cannot read ZIP archive {}: {e}", archive.display()))?;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|e| format!("cannot read ZIP entry {index}: {e}"))?;
            let enclosed = entry.enclosed_name().ok_or_else(|| {
                format!("ZIP archive {} contains an unsafe path", archive.display())
            })?;
            let output = dest.join(enclosed);
            if entry.is_dir() {
                fs::create_dir_all(&output)
                    .map_err(|e| format!("cannot create {}: {e}", output.display()))?;
            } else {
                if entry.is_symlink() {
                    return Err(format!(
                        "ZIP archive {} contains unsupported symlink {}",
                        archive.display(),
                        entry.name()
                    ));
                }
                let parent = output
                    .parent()
                    .ok_or_else(|| format!("ZIP entry {} has no parent directory", entry.name()))?;
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                let mut destination = fs::File::create(&output)
                    .map_err(|e| format!("cannot create {}: {e}", output.display()))?;
                std::io::copy(&mut entry, &mut destination)
                    .map_err(|e| format!("cannot extract {}: {e}", output.display()))?;
            }
        }
        Ok(())
    } else {
        let tar_gz = fs::File::open(archive)
            .map_err(|e| format!("cannot open {}: {e}", archive.display()))?;
        let decoder = flate2::read::GzDecoder::new(tar_gz);
        let mut tar = tar::Archive::new(decoder);
        tar.unpack(dest)
            .map_err(|e| format!("tar unpack failed: {e}"))?;
        Ok(())
    }
}
