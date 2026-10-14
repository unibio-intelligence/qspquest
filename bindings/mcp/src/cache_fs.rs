//! Filesystem checks for the private release cache.
use std::{fs, io::Read, path::Path};

#[cfg(unix)]
fn permissions(path: &Path, private: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    // geteuid has no arguments, pointers or ownership requirements.
    let uid = unsafe { geteuid() };
    let owner_ok = metadata.uid() == uid || (!private && metadata.uid() == 0);
    let shared_sticky = !private && metadata.is_dir() && metadata.mode() & 0o1000 != 0;
    let mask = if private { 0o077 } else { 0o022 };
    if !owner_ok || (metadata.mode() & mask != 0 && !shared_sticky) {
        return Err(format!("Unsafe release cache ownership or permissions: {}. Use a private directory owned by the server user (0700 on Unix).", path.display()));
    }
    if metadata.is_file() && metadata.nlink() != 1 {
        return Err("Hardlinked release cache files are forbidden".into());
    }
    Ok(())
}

#[cfg(windows)]
fn permissions(path: &Path, private: bool) -> Result<(), String> {
    windows::check(path, private)
}

#[cfg(not(any(unix, windows)))]
fn permissions(_: &Path, _: bool) -> Result<(), String> {
    Err("Release caching requires a platform with verified filesystem ownership controls".into())
}

pub(crate) fn check(path: &Path, directory: bool, private: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked || (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(format!(
            "Release cache requires regular files and directories without links: {}",
            path.display()
        ));
    }
    permissions(path, private)
}

pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok() {
        return check(path, true, true);
    }
    let parent = path
        .parent()
        .ok_or("Release cache has no parent directory")?;
    if !parent.is_dir() {
        private_directory(parent)?;
    }
    // Existing ancestors must not permit another unprivileged user to replace
    // this entry. Canonicalization allows trusted system aliases such as /tmp.
    let canonical = parent.canonicalize().map_err(|e| e.to_string())?;
    for ancestor in canonical.ancestors() {
        check(ancestor, true, false)?;
    }
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    check(path, true, true)
}

pub(crate) fn root(path: &Path) -> Result<std::path::PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if absolute
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Release cache paths must not contain '..'".into());
    }
    private_directory(&absolute)?;
    let canonical = absolute.canonicalize().map_err(|e| e.to_string())?;
    for ancestor in canonical
        .parent()
        .ok_or("Release cache cannot be a filesystem root")?
        .ancestors()
    {
        check(ancestor, true, false)?;
    }
    Ok(canonical)
}

pub(crate) fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    check(path, false, true)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err("Release cache file exceeds its size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Release cache file exceeds its size limit".into());
    }
    Ok(bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    check(path, false, true)
}

#[cfg(windows)]
mod windows {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetNamedSecurityInfoW(
            name: *const u16,
            kind: i32,
            info: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: i32,
            data: *mut c_void,
            size: u32,
            needed: *mut u32,
        ) -> i32;
        fn EqualSid(a: *mut c_void, b: *mut c_void) -> i32;
        fn IsWellKnownSid(sid: *mut c_void, kind: i32) -> i32;
        fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
        fn LookupAccountNameW(
            system: *const u16,
            account: *const u16,
            sid: *mut c_void,
            sid_size: *mut u32,
            domain: *mut u16,
            domain_size: *mut u32,
            kind: *mut i32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    struct Descriptor(*mut c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    struct Token(*mut c_void);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn installer_sid() -> Option<*mut c_void> {
        static SID: std::sync::OnceLock<Option<Vec<usize>>> = std::sync::OnceLock::new();
        SID.get_or_init(|| {
            let account: Vec<u16> = "NT SERVICE\\TrustedInstaller\0".encode_utf16().collect();
            let (mut sid_size, mut domain_size, mut kind) = (0, 0, 0);
            unsafe {
                LookupAccountNameW(
                    ptr::null(),
                    account.as_ptr(),
                    ptr::null_mut(),
                    &mut sid_size,
                    ptr::null_mut(),
                    &mut domain_size,
                    &mut kind,
                );
            }
            if !(8..=65536).contains(&sid_size) || domain_size > 65536 {
                return None;
            }
            let mut sid = vec![0usize; (sid_size as usize).div_ceil(std::mem::size_of::<usize>())];
            let mut domain = vec![0u16; domain_size as usize];
            if unsafe {
                LookupAccountNameW(
                    ptr::null(),
                    account.as_ptr(),
                    sid.as_mut_ptr().cast(),
                    &mut sid_size,
                    domain.as_mut_ptr(),
                    &mut domain_size,
                    &mut kind,
                )
            } == 0
            {
                return None;
            }
            Some(sid)
        })
        .as_ref()
        .map(|sid| sid.as_ptr() as *mut c_void)
    }
    pub(super) fn check(path: &Path, private: bool) -> Result<(), String> {
        let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
        if name.contains(&0) {
            return Err("Cache path contains NUL".into());
        }
        name.push(0);
        let (mut owner, mut dacl, mut descriptor) =
            (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
        // All returned pointers refer to the descriptor allocation until its guard drops.
        let result = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                1,
                5,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        if result != 0 {
            return Err(format!("Cannot inspect cache ACL: Windows error {result}"));
        }
        let _descriptor = Descriptor(descriptor);
        if owner.is_null() || dacl.is_null() {
            return Err("Release cache needs an owner and a non-null DACL".into());
        }
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) } == 0 {
            return Err("Cannot inspect the server user's access token".into());
        }
        let token = Token(token);
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token.0, 1, ptr::null_mut(), 0, &mut needed);
        }
        if needed < std::mem::size_of::<usize>() as u32 || needed > 65536 {
            return Err("Unexpected token-user information size".into());
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(token.0, 1, buffer.as_mut_ptr().cast(), needed, &mut needed)
        } == 0
        {
            return Err("Cannot read the server user's SID".into());
        }
        // TOKEN_USER begins with a SID pointer; its allocation remains in buffer.
        let user = buffer[0] as *mut c_void;
        let trusted = |sid| unsafe {
            EqualSid(sid, user) != 0 || IsWellKnownSid(sid, 22) != 0 || IsWellKnownSid(sid, 26) != 0
                // Protected OS ancestors may be owned by the Windows servicing account.
                // Cache entries themselves must retain the stricter user/SYSTEM/admin owner policy.
                || (!private && installer_sid().is_some_and(|installer| EqualSid(sid, installer) != 0))
        };
        if !trusted(owner) {
            return Err(format!("Unsafe release cache owner at {}: use a private directory owned by the server user, SYSTEM or Administrators", path.display()));
        }
        // ACL has an eight-byte fixed header, with AceCount at offset four.
        let count = unsafe { ptr::read_unaligned((dacl as *const u8).add(4).cast::<u16>()) };
        let mask = if private {
            0x500d_0156u32
        } else {
            0x500d_0040u32
        };
        for index in 0..u32::from(count) {
            let mut ace = ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
                return Err("Cannot inspect cache ACL entry".into());
            }
            let bytes = ace as *const u8;
            let kind = unsafe { *bytes };
            let flags = unsafe { *bytes.add(1) };
            if flags & 8 != 0 || kind == 1 {
                continue;
            }
            if kind != 0 {
                return Err("Release cache has an unsupported ACL entry".into());
            }
            let size = unsafe { ptr::read_unaligned(bytes.add(2).cast::<u16>()) };
            if size < 12 {
                return Err("Release cache has an invalid ACL entry".into());
            }
            let rights = unsafe { ptr::read_unaligned(bytes.add(4).cast::<u32>()) };
            let sid = unsafe { bytes.add(8) } as *mut c_void;
            // OWNER RIGHTS refers only to the object's already validated owner.
            // It must not be accepted as an owner identity itself.
            let owner_rights = unsafe { IsWellKnownSid(sid, 71) != 0 };
            if rights & mask != 0 && !trusted(sid) && !owner_rights {
                return Err(
                    "Release cache permits modification by another Windows principal".into(),
                );
            }
        }
        Ok(())
    }
}
