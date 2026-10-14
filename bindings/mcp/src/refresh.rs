//! Atomic, hash-pinned library updates. A failed refresh preserves the active snapshot.
use crate::library::{hash, Library};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

/// Explicit override, or the current user's application cache. Never shared temp.
pub fn cache_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("QSPQUEST_LIBRARY_CACHE") {
        if path.is_empty() {
            return Err("QSPQUEST_LIBRARY_CACHE must not be empty".into());
        }
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")));
    let base = base.filter(|p| p.is_absolute()).ok_or(
        "No absolute user cache directory; set QSPQUEST_LIBRARY_CACHE to a private directory",
    )?;
    Ok(base.join("qspquest-models"))
}

fn pin(sha: &str) -> Result<String, String> {
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Library releases require a pinned 64-character SHA-256".into());
    }
    Ok(sha.to_ascii_lowercase())
}
fn release_pin(url: &str, sha: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Library releases require an HTTPS URL")?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("Library releases require an HTTPS URL".into());
    }
    pin(sha)
}

fn release_files(bytes: &[u8], expected: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if bytes.len() > 32_000_000 || hash(bytes) != expected {
        return Err("Release tarball exceeds 32 MB or fails its pinned SHA-256".into());
    }
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(bytes)));
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for (index, entry) in archive.entries().map_err(|e| e.to_string())?.enumerate() {
        let entry = entry.map_err(|e| e.to_string())?;
        if index >= 4096 {
            return Err("Too many release members".into());
        }
        let raw = entry.path_bytes();
        let name = std::str::from_utf8(&raw).map_err(|_| "Release paths must be UTF-8")?;
        let kind = entry.header().entry_type();
        let name = if kind.is_dir() {
            name.trim_end_matches('/')
        } else {
            name
        };
        let path = Path::new(name);
        let canonical = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if name.is_empty()
            || name.contains(['\\', ':'])
            || path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || canonical != name
            || !names.insert(name.to_owned())
        {
            return Err("Duplicate or unsafe release path".into());
        }
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err("Release symlinks, hardlinks and special objects are forbidden".into());
        }
        let name = name.to_owned();
        total = total
            .checked_add(entry.size())
            .ok_or("Expanded release size overflow")?;
        if total > 64_000_000 || entry.size() > 16_000_000 {
            return Err("Expanded library exceeds limits".into());
        }
        let size = entry.size();
        let mut contents = Vec::new();
        entry
            .take(16_000_001)
            .read_to_end(&mut contents)
            .map_err(|e| e.to_string())?;
        if contents.len() as u64 != size {
            return Err("Truncated release member".into());
        }
        files.insert(name, contents);
    }
    if !files.contains_key("catalog.json") {
        return Err("Release root must contain catalog.json".into());
    }
    Ok(files)
}

fn cached(snapshot: &Path, sha: &str) -> Result<Library, String> {
    crate::cache_fs::check(snapshot, true, true)?;
    let mut root_names = std::fs::read_dir(snapshot)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.file_name()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    root_names.sort();
    if root_names
        != [
            std::ffi::OsString::from("library"),
            std::ffi::OsString::from("release.tar.gz"),
        ]
    {
        return Err("Cached release must retain its pinned archive and library inventory; use a new private cache directory for legacy entries".into());
    }
    let bytes = crate::cache_fs::read(&snapshot.join("release.tar.gz"), 32_000_000)?;
    let files = release_files(&bytes, sha)?;
    let root = snapshot.join("library");
    crate::cache_fs::check(&root, true, true)?;
    let mut expected_dirs = BTreeSet::new();
    for name in files.keys() {
        for parent in Path::new(name)
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            expected_dirs.insert(parent.to_path_buf());
        }
    }
    let mut pending = vec![root.clone()];
    let mut found = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let relative = path.strip_prefix(&root).map_err(|e| e.to_string())?;
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                crate::cache_fs::check(&path, true, true)?;
                if !expected_dirs.contains(relative) {
                    return Err("Unexpected cached release directory".into());
                }
                pending.push(path);
            } else {
                let name = relative
                    .to_str()
                    .ok_or("Cached release path is not UTF-8")?
                    .replace('\\', "/");
                let expected = files.get(&name).ok_or("Unexpected cached release file")?;
                let actual = crate::cache_fs::read(&path, 16_000_000)?;
                if &actual != expected {
                    return Err(format!(
                        "Cached release differs from pinned archive: {name}"
                    ));
                }
                found.insert(name);
            }
        }
    }
    if found.len() != files.len() {
        return Err("Cached release inventory is incomplete".into());
    }
    // Load only authenticated bytes. Later on-disk changes cannot change this snapshot.
    Library::verified_files(&files)
}

pub fn unpack(bytes: &[u8], expected: &str, cache: &Path) -> Result<Library, String> {
    let expected = pin(expected)?;
    let files = release_files(bytes, &expected)?;
    let library = Library::verified_files(&files)?;
    let cache = crate::cache_fs::root(cache)?;
    let final_dir = cache.join(&expected);
    if std::fs::symlink_metadata(&final_dir).is_ok() {
        return cached(&final_dir, &expected);
    }
    let stage = (0..16)
        .find_map(|_| {
            let path = cache.join(format!(
                ".{expected}-{}-{}",
                std::process::id(),
                STAGE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            if std::fs::symlink_metadata(&path).is_ok() {
                return None;
            }
            Some(path)
        })
        .ok_or("Release staging names already exist")?;
    let result = (|| {
        crate::cache_fs::private_directory(&stage)?;
        crate::cache_fs::write_new(&stage.join("release.tar.gz"), bytes)?;
        let root = stage.join("library");
        crate::cache_fs::private_directory(&root)?;
        for (name, bytes) in &files {
            let path = root.join(name);
            crate::cache_fs::private_directory(
                path.parent().ok_or("Missing release member parent")?,
            )?;
            crate::cache_fs::write_new(&path, bytes)?;
        }
        // No file in a verified release is changed in place.
        match std::fs::rename(&stage, &final_dir) {
            Ok(()) => Ok(library),
            Err(error) => {
                if std::fs::symlink_metadata(&final_dir).is_ok() {
                    cached(&final_dir, &expected)
                } else {
                    Err(error.to_string())
                }
            }
        }
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

pub async fn refresh(url: &str, sha: &str, cache: &Path) -> Result<Library, String> {
    let sha = release_pin(url, sha)?;
    let cache = crate::cache_fs::root(cache)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let mut r = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 32_000_000 {
            return Err("Release download exceeds 32 MB".into());
        }
        bytes.extend_from_slice(&chunk)
    }
    unpack(&bytes, &sha, &cache)
}
/// Load a verified cached release, or fetch the pinned release before serving.
pub async fn initial(url: &str, sha: &str, cache: &Path) -> Result<Library, String> {
    let sha = release_pin(url, sha)?;
    let cache = crate::cache_fs::root(cache)?;
    let snapshot = cache.join(&sha);
    if std::fs::symlink_metadata(&snapshot).is_ok() {
        return cached(&snapshot, &sha);
    }
    refresh(url, &sha, &cache).await
}

pub fn start(library: Arc<RwLock<Library>>) {
    let Ok(url) = std::env::var("QSPQUEST_LIBRARY_RELEASE_URL") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let Ok(sha) = std::env::var("QSPQUEST_LIBRARY_RELEASE_SHA256") else {
        eprintln!("Library refresh disabled: release SHA-256 is required");
        return;
    };
    let cache = match cache_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Library refresh disabled: {error}");
            return;
        }
    };
    let interval = std::env::var("QSPQUEST_LIBRARY_REFRESH_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600)
        .max(60);
    tokio::spawn(async move {
        loop {
            match refresh(&url, &sha, &cache).await {
                Ok(lib) => {
                    *library.write().unwrap() = lib;
                }
                Err(e) => eprintln!("Library refresh rejected; retaining current snapshot: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    });
}
