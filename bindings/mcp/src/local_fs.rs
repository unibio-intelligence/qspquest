//! Anchored private files for explicitly configured local result storage.
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

pub(crate) struct PrivateDirectory {
    path: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    anchor: File,
    #[cfg(windows)]
    _anchors: Vec<File>,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        || name == "."
        || name == ".."
    {
        return Err(invalid(
            "Result storage accepts only internal ordinary filenames",
        ));
    }
    Ok(())
}
impl PrivateDirectory {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let path = crate::cache_fs::root(path).map_err(invalid)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let anchor = OpenOptions::new()
                .read(true)
                .custom_flags(unix::DIRECTORY | unix::NOFOLLOW)
                .open(&path)?;
            Ok(Self { path, anchor })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let mut parents: Vec<_> = path.ancestors().collect();
            parents.reverse();
            let mut anchors = Vec::new();
            for parent in parents {
                let file = OpenOptions::new()
                    .access_mode(1)
                    .share_mode(1)
                    .custom_flags(0x02000000 | 0x00200000)
                    .open(parent)?;
                let metadata = file.metadata()?;
                use std::os::windows::fs::MetadataExt;
                if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                    return Err(invalid("Result storage parent is not a regular directory"));
                }
                anchors.push(file);
            }
            Ok(Self {
                path,
                _anchors: anchors,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = path;
            Err(invalid(
                "Safe result storage requires Linux, macOS or Windows",
            ))
        }
    }
    fn check_root(&self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = self.anchor.metadata()?;
            if metadata.uid() != unix::uid() || metadata.mode() & 0o077 != 0 {
                return Err(invalid(
                    "Result storage directory must remain private and owned",
                ));
            }
        }
        #[cfg(windows)]
        crate::cache_fs::check(&self.path, true, true).map_err(invalid)?;
        Ok(())
    }
    pub(crate) fn file(
        &self,
        filename: &str,
        create: bool,
        writable: bool,
        exclusive: bool,
    ) -> io::Result<File> {
        self.check_root()?;
        name(filename)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let file = unix::file(&self.anchor, filename, create, writable)?;
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            OpenOptions::new()
                .read(true)
                .write(writable)
                .create_new(create)
                .share_mode(if exclusive { 0 } else { 1 })
                .custom_flags(0x00200000)
                .open(self.path.join(filename))?
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        let file: File = return Err(invalid("Safe result storage is unavailable"));
        self.check_file(filename, &file)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if exclusive {
            unix::lock(&file)?;
        }
        Ok(file)
    }
    fn check_file(&self, filename: &str, file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid("Result storage requires regular files"));
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = filename;
            if metadata.uid() != unix::uid()
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
            {
                return Err(invalid(
                    "Result storage files must be private, owned and not hardlinked",
                ));
            }
        }
        #[cfg(windows)]
        {
            windows::check_file(file)?;
            crate::cache_fs::check(&self.path.join(filename), false, true).map_err(invalid)?;
        }
        Ok(())
    }
    pub(crate) fn names(&self) -> io::Result<Vec<String>> {
        self.check_root()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let same_root = || -> io::Result<()> {
            use std::os::unix::fs::MetadataExt;
            let held = self.anchor.metadata()?;
            let current = std::fs::symlink_metadata(&self.path)?;
            if !current.is_dir() || (current.dev(), current.ino()) != (held.dev(), held.ino()) {
                return Err(invalid(
                    "The result storage directory moved; reopen its original location",
                ));
            }
            Ok(())
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        same_root()?;
        let names = std::fs::read_dir(&self.path)?
            .map(|entry| {
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("Unexpected non-UTF8 result storage filename"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        same_root()?;
        Ok(names)
    }
    pub(crate) fn remove(&self, filename: &str) -> io::Result<()> {
        let file = self.file(filename, false, false, false)?;
        drop(file);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        return unix::remove(&self.anchor, filename);
        #[cfg(windows)]
        return std::fs::remove_file(self.path.join(filename));
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        Err(invalid("Safe result storage is unavailable"))
    }
    pub(crate) fn sync(&self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        self.anchor.sync_all()?;
        Ok(())
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    #[cfg(all(
        target_os = "linux",
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ))]
    compile_error!(
        "Safe local storage supports Linux x86_64 and aarch64; other architectures require native open flags"
    );
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(super) const DIRECTORY: i32 = 0x10000;
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(super) const NOFOLLOW: i32 = 0x20000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    pub(super) const DIRECTORY: i32 = 0x4000;
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    pub(super) const NOFOLLOW: i32 = 0x8000;
    #[cfg(target_os = "macos")]
    pub(super) const DIRECTORY: i32 = 0x100000;
    #[cfg(target_os = "macos")]
    pub(super) const NOFOLLOW: i32 = 0x100;
    #[cfg(target_os = "linux")]
    const EXCLUSIVE: i32 = 0x40 | 0x80 | 0x80000;
    #[cfg(target_os = "linux")]
    const NONBLOCK: i32 = 0x800;
    #[cfg(target_os = "linux")]
    const CLOEXEC: i32 = 0x80000;
    #[cfg(target_os = "macos")]
    const EXCLUSIVE: i32 = 0x200 | 0x800 | 0x1000000;
    #[cfg(target_os = "macos")]
    const NONBLOCK: i32 = 0x4;
    #[cfg(target_os = "macos")]
    const CLOEXEC: i32 = 0x1000000;
    unsafe extern "C" {
        fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
        fn unlinkat(fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
        fn flock(fd: i32, operation: i32) -> i32;
        fn geteuid() -> u32;
    }
    pub(super) fn uid() -> u32 {
        unsafe { geteuid() }
    }
    pub(super) fn file(
        anchor: &File,
        name: &str,
        create: bool,
        writable: bool,
    ) -> io::Result<File> {
        let name = CString::new(name).map_err(|_| invalid("NUL in storage filename"))?;
        // One validated component; O_NONBLOCK prevents a substituted FIFO from blocking.
        let flags = CLOEXEC
            | NOFOLLOW
            | NONBLOCK
            | if writable { 2 } else { 0 }
            | if create { EXCLUSIVE } else { 0 };
        let fd = unsafe { openat(anchor.as_raw_fd(), name.as_ptr(), flags, 0o600u32) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub(super) fn remove(anchor: &File, name: &str) -> io::Result<()> {
        let name = CString::new(name).map_err(|_| invalid("NUL in storage filename"))?;
        // unlinkat removes an anchored name, never a link target.
        if unsafe { unlinkat(anchor.as_raw_fd(), name.as_ptr(), 0) } < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub(super) fn lock(file: &File) -> io::Result<()> {
        // LOCK_EX | LOCK_NB; the descriptor holds the lock until store shutdown.
        if unsafe { flock(file.as_raw_fd(), 2 | 4) } < 0 {
            Err(invalid("Result storage is already open by another server"))
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{invalid, io, File};
    use std::{ffi::c_void, mem::MaybeUninit, os::windows::io::AsRawHandle};

    #[repr(C)]
    struct FileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut c_void,
            information: *mut FileInformation,
        ) -> i32;
    }
    pub(super) fn check_file(file: &File) -> io::Result<()> {
        let mut information = MaybeUninit::<FileInformation>::uninit();
        // The owned handle remains live; a successful call initializes this C-layout record.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let information = unsafe { information.assume_init() };
        if information.links != 1 || information.attributes & (0x400 | 0x10) != 0 {
            return Err(invalid(
                "Result storage requires regular files without hardlinks or reparse points",
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod platform_flag_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::fs::symlink,
    };

    #[test]
    fn native_storage_flags_preserve_regular_files_and_refuse_links() {
        let root = std::env::temp_dir().join(format!(
            "qspquest-storage-open-flags-{}",
            std::process::id()
        ));
        let directory = PrivateDirectory::open(&root).unwrap();
        assert!(directory.anchor.metadata().unwrap().is_dir());
        let mut created = directory.file("result.json", true, true, true).unwrap();
        created.write_all(b"original result").unwrap();
        created.sync_all().unwrap();
        drop(created);
        let mut result = String::new();
        directory
            .file("result.json", false, false, false)
            .unwrap()
            .read_to_string(&mut result)
            .unwrap();
        assert_eq!(result, "original result");
        assert!(directory.file("result.json", true, true, false).is_err());
        symlink(root.join("result.json"), root.join("linked.json")).unwrap();
        assert!(directory.file("linked.json", false, false, false).is_err());
        assert!(directory.file("linked.json", false, true, false).is_err());
        std::fs::create_dir(root.join("subdirectory")).unwrap();
        assert!(directory.file("subdirectory", false, false, false).is_err());
        symlink(root.join("subdirectory"), root.join("linked-directory")).unwrap();
        assert!(directory
            .file("linked-directory", false, false, false)
            .is_err());
        assert_eq!(
            std::fs::read(root.join("result.json")).unwrap(),
            b"original result"
        );
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }
}
