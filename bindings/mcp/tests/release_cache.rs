use qspquest_mcp::{
    library::{hash, Library},
    refresh,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "qspquest-cache-test-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
    fn cache(&self) -> PathBuf {
        self.0.join("cache")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn files() -> BTreeMap<String, Vec<u8>> {
    let model = include_bytes!(
        "../assets/library/models/qspquest_diffdose_2026_bite_benchmark/0.1.0/model.omex"
    )
    .to_vec();
    let catalog = serde_json::to_vec(&json!({"models":[{"id":"qspquest_diffdose_2026_bite_benchmark", "version":"0.1.0", "archive":"models/bite.omex", "omex_sha256":hash(&model), "scenario_ids":[], "fit_ready":false, "fitting_supported":false}]})).unwrap();
    BTreeMap::from([
        ("catalog.json".into(), catalog),
        ("models/bite.omex".into(), model),
        ("README.md".into(), b"Synthetic cache test fixture".to_vec()),
    ])
}
fn tar(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(contents.len() as u64);
        header.set_cksum();
        archive
            .append_data(&mut header, path, contents.as_slice())
            .unwrap();
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&archive.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}
#[tokio::test]
async fn offline_reuse_is_bound_to_release_not_a_self_consistent_catalog() {
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    let installed = refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
    let cached = refresh::initial(
        "https://invalid.example/release.tar.gz",
        &sha,
        &scratch.cache(),
    )
    .await
    .unwrap();
    assert_eq!(cached.version, installed.version);
    let catalog = scratch.cache().join(&sha).join("library/catalog.json");
    let mut changed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&catalog).unwrap()).unwrap();
    changed["models"][0]["name"] = json!("Untrusted replacement");
    std::fs::write(&catalog, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(Library::directory(catalog.parent().unwrap()).is_ok());
    let error = refresh::initial(
        "https://invalid.example/release.tar.gz",
        &sha,
        &scratch.cache(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("differs from pinned archive"), "{error}");
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache()).is_err());
}
#[tokio::test]
async fn precreated_unrelated_snapshot_cannot_satisfy_pin() {
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    let snapshot = scratch.cache().join(&sha);
    std::fs::create_dir_all(&snapshot).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [scratch.cache(), snapshot.clone()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    std::fs::write(snapshot.join("catalog.json"), br#"{"models":[]}"#).unwrap();
    assert!(Library::directory(&snapshot).is_ok());
    let error = refresh::initial(
        "https://invalid.example/release.tar.gz",
        &sha,
        &scratch.cache(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("retain its pinned archive"), "{error}");
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache()).is_err());
    assert!(snapshot.join("catalog.json").is_file());
}
#[tokio::test]
async fn changed_archive_model_and_inventory_are_refused() {
    for member in [
        "release.tar.gz",
        "library/models/bite.omex",
        "library/README.md",
    ] {
        let scratch = Scratch::new();
        let bytes = tar(&files());
        let sha = hash(&bytes);
        refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
        let path = scratch.cache().join(&sha).join(member);
        let mut contents = std::fs::read(&path).unwrap();
        contents.push(0);
        std::fs::write(&path, contents).unwrap();
        assert!(
            refresh::initial(
                "https://invalid.example/release.tar.gz",
                &sha,
                &scratch.cache()
            )
            .await
            .is_err(),
            "{member}"
        );
    }
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
    let root = scratch.cache().join(&sha).join("library");
    std::fs::write(root.join("extra.json"), b"{}").unwrap();
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache()).is_err());
    std::fs::remove_file(root.join("extra.json")).unwrap();
    std::fs::remove_file(root.join("README.md")).unwrap();
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache()).is_err());
}
#[test]
fn failed_refresh_and_abandoned_staging_preserve_previous_release() {
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    let old = refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
    let abandoned = scratch.cache().join(".interrupted-refresh");
    std::fs::create_dir(&abandoned).unwrap();
    std::fs::write(abandoned.join("partial.tar.gz"), b"interrupted").unwrap();
    let broken = tar(&BTreeMap::from([(
        "catalog.json".into(),
        b"invalid".to_vec(),
    )]));
    assert!(refresh::unpack(&broken, &hash(&broken), &scratch.cache()).is_err());
    assert_eq!(
        refresh::unpack(&bytes, &sha, &scratch.cache())
            .unwrap()
            .version,
        old.version
    );
    assert!(abandoned.join("partial.tar.gz").is_file());
    let mut updated = files();
    updated.insert("README.md".into(), b"New pinned release".to_vec());
    let new_bytes = tar(&updated);
    let new_sha = hash(&new_bytes);
    refresh::unpack(&new_bytes, &new_sha, &scratch.cache()).unwrap();
    assert_eq!(
        std::fs::read(scratch.cache().join(sha).join("library/README.md")).unwrap(),
        b"Synthetic cache test fixture"
    );
    assert_eq!(
        std::fs::read(scratch.cache().join(new_sha).join("library/README.md")).unwrap(),
        b"New pinned release"
    );
}
#[cfg(unix)]
#[test]
fn private_permissions_and_symlinks_are_enforced_in_shared_temp() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    std::fs::create_dir(scratch.cache()).unwrap();
    std::fs::set_permissions(scratch.cache(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache())
        .err()
        .unwrap()
        .contains("permissions"));
    std::fs::set_permissions(scratch.cache(), std::fs::Permissions::from_mode(0o700)).unwrap();
    refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
    let alias = scratch.0.join("alias");
    symlink(scratch.cache(), &alias).unwrap();
    assert!(refresh::unpack(&bytes, &sha, &alias).is_err());
    let path = scratch.cache().join(&sha).join("library/README.md");
    std::fs::remove_file(&path).unwrap();
    let outside = scratch.0.join("outside.txt");
    std::fs::write(&outside, b"Synthetic cache test fixture").unwrap();
    symlink(&outside, &path).unwrap();
    assert!(refresh::unpack(&bytes, &sha, &scratch.cache()).is_err());
    assert_eq!(
        std::fs::read(outside).unwrap(),
        b"Synthetic cache test fixture"
    );
}

#[test]
fn concurrent_publishers_obtain_one_verified_immutable_release() {
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for _ in 0..4 {
            let barrier = barrier.clone();
            let (bytes, sha, cache) = (&bytes, &sha, scratch.cache());
            threads.push(scope.spawn(move || {
                barrier.wait();
                refresh::unpack(bytes, sha, &cache).unwrap().version
            }));
        }
        let expected = threads.remove(0).join().unwrap();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), expected);
        }
    });
    assert_eq!(std::fs::read_dir(scratch.cache()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn foreign_owned_private_cache_is_refused() {
    let Some(path) = std::env::var_os("QSPQUEST_TEST_FOREIGN_CACHE") else {
        println!("Foreign-owner fixture is supplied by the Linux platform test runner");
        return;
    };
    use std::os::unix::fs::MetadataExt;
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    let path = PathBuf::from(path);
    let metadata = std::fs::metadata(&path).unwrap();
    assert_ne!(metadata.uid(), unsafe { geteuid() });
    assert_eq!(metadata.mode() & 0o777, 0o700);
    let bytes = tar(&files());
    let error = refresh::unpack(&bytes, &hash(&bytes), &path).err().unwrap();
    assert!(error.contains("ownership or permissions"), "{error}");
}

#[cfg(windows)]
#[test]
fn another_windows_principal_cannot_write_the_cache() {
    let scratch = Scratch::new();
    let bytes = tar(&files());
    let sha = hash(&bytes);
    refresh::unpack(&bytes, &sha, &scratch.cache()).unwrap();
    let command =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/icacls.exe");
    let status = std::process::Command::new(command)
        .arg(scratch.cache())
        .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
        .status()
        .unwrap();
    assert!(status.success());
    let error = refresh::unpack(&bytes, &sha, &scratch.cache())
        .err()
        .unwrap();
    assert!(error.contains("another Windows principal"), "{error}");
}
