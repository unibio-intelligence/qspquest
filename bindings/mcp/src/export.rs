//! Explicit local CSV exports beneath an opened private output directory.
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

pub fn csv_path(path: &str) -> Result<PathBuf, String> {
    let parsed = Path::new(path);
    let parts: Vec<_> = path.split('/').collect();
    if path.is_empty()
        || parsed.is_absolute()
        || parsed.extension().is_none_or(|e| e != "csv")
        || parts.iter().any(|part| {
            let stem = part.split('.').next().unwrap_or("").to_uppercase();
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|s| {
                        matches!(
                            s,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                })
        })
    {
        return Err("save_csv must be a relative .csv path beneath the configured output directory, using ordinary names and '/' separators, without '..', links or device names".into());
    }
    Ok(parsed.to_path_buf())
}

pub struct OutputDirectory {
    path: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    anchor: File,
    #[cfg(windows)]
    _anchors: Vec<File>,
}
impl OutputDirectory {
    pub fn open(path: &Path) -> Result<Self, String> {
        let path =
            crate::cache_fs::root(path).map_err(|e| format!("Unsafe CSV output directory: {e}"))?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let anchor = unix::directory(&path)?;
            Ok(Self { path, anchor })
        }
        #[cfg(windows)]
        {
            let mut ancestors: Vec<_> = path.ancestors().collect();
            ancestors.reverse();
            let mut anchors = Vec::new();
            for directory in ancestors {
                anchors.push(windows::directory(directory)?);
            }
            Ok(Self {
                path,
                _anchors: anchors,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = path;
            Err("Safe CSV exports are supported on Linux, macOS and Windows".into())
        }
    }
    /// Read-only admission of both new names. This reserves nothing; writes recheck exclusively.
    pub fn preflight_bundle(&self, relative: &str) -> Result<(), String> {
        let relative = csv_path(relative)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        return unix::preflight(&self.anchor, &relative);
        #[cfg(windows)]
        return windows::preflight(&self.path, &relative);
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        Err("Safe CSV exports are unavailable on this platform".into())
    }
    /// Creates a new file; existing outputs, including links, are never replaced.
    pub fn write_csv(&self, relative: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let relative = csv_path(relative)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        unix::write(&self.anchor, &relative, bytes, None)?;
        #[cfg(windows)]
        windows::write(&self.path, &relative, bytes, None)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        return Err("Safe CSV exports are unavailable on this platform".into());
        Ok(self.path.join(relative))
    }
    /// Reserve both names before writing; incomplete pairs are never successful exports.
    pub fn write_bundle(
        &self,
        relative: &str,
        csv: &[u8],
        manifest: &[u8],
    ) -> Result<(PathBuf, PathBuf), String> {
        let relative = csv_path(relative)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        unix::write(&self.anchor, &relative, csv, Some(manifest))?;
        #[cfg(windows)]
        windows::write(&self.path, &relative, csv, Some(manifest))?;
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        return Err("Safe CSV exports are unavailable on this platform".into());
        let mut sidecar = relative.as_os_str().to_os_string();
        sidecar.push(".manifest.json");
        Ok((self.path.join(relative), self.path.join(sidecar)))
    }
}

fn finish(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    fail_write()?;
    file.write_all(bytes)?;
    #[cfg(test)]
    fail_write()?;
    file.sync_all()
}

#[cfg(test)]
thread_local! {
    static FAIL_WRITE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
fn fail_write() -> std::io::Result<()> {
    FAIL_WRITE.with(|state| match state.get() {
        Some(0) => {
            state.set(None);
            Err(std::io::Error::other("synthetic write/sync failure"))
        }
        Some(n) => {
            state.set(Some(n - 1));
            Ok(())
        }
        None => Ok(()),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
    };
    #[cfg(all(
        target_os = "linux",
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ))]
    compile_error!(
        "Safe local storage supports Linux x86_64 and aarch64; other architectures require native open flags"
    );
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const DIRECTORY: i32 = 0x10000;
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const NOFOLLOW: i32 = 0x20000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const DIRECTORY: i32 = 0x4000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    const NOFOLLOW: i32 = 0x8000;
    #[cfg(target_os = "macos")]
    const DIRECTORY: i32 = 0x100000;
    #[cfg(target_os = "macos")]
    const NOFOLLOW: i32 = 0x100;
    #[cfg(target_os = "linux")]
    const CREATE_EXCLUSIVE: i32 = 0x40 | 0x80 | 0x80000;
    #[cfg(target_os = "macos")]
    const CREATE_EXCLUSIVE: i32 = 0x200 | 0x800 | 0x1000000;
    unsafe extern "C" {
        fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
        fn mkdirat(fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
        fn unlinkat(fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
        fn geteuid() -> u32;
        fn readlinkat(
            fd: i32,
            path: *const std::ffi::c_char,
            buffer: *mut std::ffi::c_char,
            size: usize,
        ) -> isize;
    }
    pub(super) fn directory(path: &Path) -> Result<File, String> {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(DIRECTORY | NOFOLLOW)
            .open(path)
            .map_err(|e| format!("Cannot open CSV output directory without following links: {e}"))
    }
    fn open_child(parent: &File, name: &CString, flags: i32) -> Result<File, std::io::Error> {
        // Names contain one validated component. The returned descriptor is owned here.
        let fd = unsafe { openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600u32) };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub(super) fn preflight(anchor: &File, relative: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let mut parent = anchor.try_clone().map_err(|e| e.to_string())?;
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            let metadata = parent.metadata().map_err(|e| e.to_string())?;
            if metadata.uid() != unsafe { geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.mode() & 0o300 != 0o300
            {
                return Err(
                    "CSV output directories must be owned by the server user with mode 0700".into(),
                );
            }
            let name =
                CString::new(part.as_os_str().as_bytes()).map_err(|_| "CSV path contains NUL")?;
            if parts.peek().is_none() {
                let sidecar = CString::new(format!(
                    "{}.manifest.json",
                    part.as_os_str().to_string_lossy()
                ))
                .map_err(|_| "CSV path contains NUL")?;
                for name in [name, sidecar] {
                    // readlinkat checks the anchored name without following links or opening files/devices.
                    let mut byte = 0;
                    let result =
                        unsafe { readlinkat(parent.as_raw_fd(), name.as_ptr(), &mut byte, 1) };
                    if result >= 0
                        || std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
                    {
                        return Err("CSV bundle requires two new names without links; choose a new filename".into());
                    }
                }
                return Ok(());
            }
            parent = match open_child(&parent, &name, DIRECTORY | NOFOLLOW) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "CSV parent must be a directory without links: {error}"
                    ))
                }
            };
        }
        Err("CSV filename is missing".into())
    }
    pub(super) fn write(
        anchor: &File,
        relative: &Path,
        bytes: &[u8],
        manifest: Option<&[u8]>,
    ) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let mut parent = anchor.try_clone().map_err(|e| e.to_string())?;
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            let metadata = parent.metadata().map_err(|e| e.to_string())?;
            if metadata.uid() != unsafe { geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(
                    "CSV output directories must be owned by the server user with mode 0700".into(),
                );
            }
            let name =
                CString::new(part.as_os_str().as_bytes()).map_err(|_| "CSV path contains NUL")?;
            if parts.peek().is_none() {
                let mut payloads = vec![(name, bytes)];
                if let Some(metadata) = manifest {
                    let name = CString::new(format!(
                        "{}.manifest.json",
                        part.as_os_str().to_string_lossy()
                    ))
                    .map_err(|_| "CSV path contains NUL")?;
                    payloads.push((name, metadata));
                }
                let mut created = Vec::new();
                let result: Result<(), String> = (|| {
                    for (name, payload) in payloads {
                        let file = open_child(&parent, &name, 1 | CREATE_EXCLUSIVE | NOFOLLOW)
                            .map_err(|e| format!("Cannot create CSV bundle; choose a new filename if either file exists: {e}"))?;
                        created.push((name, file, payload));
                    }
                    for (_, file, payload) in &mut created {
                        finish(file, payload)
                            .map_err(|e| format!("CSV bundle could not be completed: {e}"))?;
                    }
                    Ok(())
                })();
                if result.is_err() {
                    // Both entries use the same anchored parent; existing files were never opened.
                    for (name, _, _) in &created {
                        unsafe {
                            unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                        }
                    }
                }
                return result;
            }
            let opened = open_child(&parent, &name, DIRECTORY | NOFOLLOW);
            parent = match opened {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
                    if result != 0
                        && std::io::Error::last_os_error().kind()
                            != std::io::ErrorKind::AlreadyExists
                    {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    open_child(&parent, &name, DIRECTORY | NOFOLLOW).map_err(|e| e.to_string())?
                }
                Err(error) => {
                    return Err(format!(
                        "CSV parent must be a directory without links: {error}"
                    ))
                }
            };
        }
        Err("CSV filename is missing".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    pub(super) fn directory(path: &Path) -> Result<File, String> {
        // FILE_LIST_DIRECTORY participates in sharing checks; metadata-only access does not.
        // Keep every ancestor open without write or delete sharing while writing.
        let file = std::fs::OpenOptions::new()
            .access_mode(1)
            .share_mode(1)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("CSV parent must be a directory without links or reparse points".into());
        }
        Ok(file)
    }
    pub(super) fn preflight(root: &Path, relative: &Path) -> Result<(), String> {
        let mut path = root.to_path_buf();
        let mut anchors = Vec::new();
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            path.push(part.as_os_str());
            if parts.peek().is_none() {
                let mut sidecar = path.as_os_str().to_os_string();
                sidecar.push(".manifest.json");
                for name in [&path, &PathBuf::from(sidecar)] {
                    match std::fs::symlink_metadata(name) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                        _ => return Err("CSV bundle requires two new names without links; choose a new filename".into()),
                    }
                }
                return Ok(());
            }
            match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
                Ok(_) => {}
            }
            anchors.push(directory(&path)?);
            crate::cache_fs::check(&path, true, true)?;
        }
        Err("CSV filename is missing".into())
    }
    pub(super) fn write(
        root: &Path,
        relative: &Path,
        bytes: &[u8],
        manifest: Option<&[u8]>,
    ) -> Result<(), String> {
        let mut path = root.to_path_buf();
        let mut anchors = Vec::new();
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            path.push(part.as_os_str());
            if parts.peek().is_none() {
                let mut payloads = vec![(path.clone(), bytes)];
                if let Some(metadata) = manifest {
                    let mut sidecar = path.as_os_str().to_os_string();
                    sidecar.push(".manifest.json");
                    payloads.push((PathBuf::from(sidecar), metadata));
                }
                let mut created = Vec::new();
                let result: Result<(), String> = (|| {
                    for (path, payload) in payloads {
                        let file = std::fs::OpenOptions::new().write(true).create_new(true).share_mode(0)
                            .custom_flags(0x00200000).open(&path)
                            .map_err(|e| format!("Cannot create CSV bundle; choose a new filename if either file exists: {e}"))?;
                        created.push((path, file, payload));
                    }
                    for (_, file, payload) in &mut created {
                        finish(file, payload)
                            .map_err(|e| format!("CSV bundle could not be completed: {e}"))?;
                    }
                    Ok(())
                })();
                if result.is_err() {
                    let names: Vec<_> = created.iter().map(|(name, _, _)| name.clone()).collect();
                    drop(created);
                    for name in names {
                        let _ = std::fs::remove_file(name);
                    }
                }
                return result;
            }
            match std::fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
            anchors.push(directory(&path)?);
            crate::cache_fs::check(&path, true, true)?;
        }
        Err("CSV filename is missing".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_preflight_creates_nothing_and_refuses_either_existing_name() {
        let root =
            std::env::temp_dir().join(format!("qspquest-bundle-preflight-{}", std::process::id()));
        let output = OutputDirectory::open(&root).unwrap();
        output.preflight_bundle("nested/new.csv").unwrap();
        assert!(!root.join("nested").exists());
        output.preflight_bundle("new.csv").unwrap();
        std::fs::write(root.join("new.csv.manifest.json"), "existing").unwrap();
        assert!(output.preflight_bundle("new.csv").is_err());
        assert!(!root.join("new.csv").exists());
        std::fs::write(root.join("blocked"), "file").unwrap();
        assert!(output.preflight_bundle("blocked/new.csv").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing", root.join("link.csv")).unwrap();
            assert!(output.preflight_bundle("link.csv").is_err());
            std::os::unix::fs::symlink(&root, root.join("parent")).unwrap();
            assert!(output.preflight_bundle("parent/new.csv").is_err());
        }
        drop(output);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_bundle_write_or_sync_removes_both_reservations() {
        let root =
            std::env::temp_dir().join(format!("qspquest-bundle-fault-{}", std::process::id()));
        let output = OutputDirectory::open(&root).unwrap();
        for failure in 0..4 {
            FAIL_WRITE.with(|state| state.set(Some(failure)));
            let error = output
                .write_bundle("fault.csv", b"data", b"manifest")
                .unwrap_err();
            assert!(error.contains("synthetic write/sync failure"));
            assert!(!root.join("fault.csv").exists());
            assert!(!root.join("fault.csv.manifest.json").exists());
        }
        FAIL_WRITE.with(|state| state.set(None));
        output
            .write_bundle("good.csv", b"data", b"manifest")
            .unwrap();
        assert_eq!(std::fs::read(root.join("good.csv")).unwrap(), b"data");
        assert_eq!(
            std::fs::read(root.join("good.csv.manifest.json")).unwrap(),
            b"manifest"
        );
        drop(output);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod platform_flag_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn native_directory_flags_open_directories_and_refuse_files_and_links() {
        let root =
            std::env::temp_dir().join(format!("qspquest-export-open-flags-{}", std::process::id()));
        let output = OutputDirectory::open(&root).unwrap();
        assert!(unix::directory(&root).unwrap().metadata().unwrap().is_dir());
        output
            .write_csv("nested/result.csv", b"time,value\n0,1\n")
            .unwrap();
        let file = root.join("nested/result.csv");
        assert_eq!(std::fs::read(&file).unwrap(), b"time,value\n0,1\n");
        assert!(unix::directory(&file).is_err());
        symlink(root.join("nested"), root.join("linked-directory")).unwrap();
        assert!(unix::directory(&root.join("linked-directory")).is_err());
        assert!(output
            .write_csv("linked-directory/redirected.csv", b"bad")
            .is_err());
        assert!(!root.join("nested/redirected.csv").exists());
        symlink(&file, root.join("linked-file.csv")).unwrap();
        assert!(output.write_csv("linked-file.csv", b"bad").is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"time,value\n0,1\n");
        drop(output);
        std::fs::remove_dir_all(root).unwrap();
    }
}
