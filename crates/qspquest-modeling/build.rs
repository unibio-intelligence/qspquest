use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
fn sources(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|s| s == "rs") {
            files.push(path);
        }
    }
}
fn relative_source_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("source path within workspace")
        .components()
        .map(|part| part.as_os_str().to_str().expect("UTF-8 source path"))
        .collect::<Vec<_>>()
        .join("/")
}
/// Diffsol identity from the QSPQuest workspace lockfile. Our builds patch
/// Diffsol to an exact upstream Git revision; an unpatched build uses crates.io.
fn locked_diffsol(root: &Path) -> (String, bool) {
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("locked dependency identities");
    let dependency = lock
        .split("[[package]]")
        .find(|block| block.lines().any(|line| line == "name = \"diffsol\""))
        .expect("locked Diffsol dependency");
    let field = |name: &str| {
        dependency
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name} = \"")))
            .and_then(|value| value.strip_suffix('"'))
            .expect("locked Diffsol source field")
    };
    let version = field("version");
    let source = field("source");
    if source == "registry+https://github.com/rust-lang/crates.io-index" {
        return (
            format!("{version}+crates-io"),
            version_at_least(version, 0, 19),
        );
    }
    assert!(
        source.starts_with("git+https://github.com/martinjrobins/diffsol?rev="),
        "Diffsol must come from crates.io or the approved upstream repository"
    );
    let revision = source
        .rsplit_once('#')
        .map(|(_, revision)| revision)
        .filter(|revision| revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()))
        .expect("Diffsol Git source must name an exact revision");
    // The pinned revisions postdate 0.18.0 and provide the step limit.
    (format!("{version}+git:{revision}"), true)
}
fn version_at_least(version: &str, major: u64, minor: u64) -> bool {
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= (major, minor)
}
/// Inside the QSPQuest workspace, hash every input of the engine build.
fn workspace_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for directory in [
        "crates/qspquest-core/src",
        "crates/qspquest-modeling/src",
        "bindings/wasm/src",
    ] {
        let dir = root.join(directory);
        println!("cargo:rerun-if-changed={}", dir.display());
        sources(&dir, &mut files);
    }
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/config.toml",
        "crates/qspquest-core/Cargo.toml",
        "crates/qspquest-modeling/Cargo.toml",
        "bindings/wasm/Cargo.toml",
        "bindings/mcp/src/core/compute.rs",
        "crates/qspquest-modeling/build.rs",
    ] {
        files.push(root.join(file));
    }
    files
}
fn main() {
    println!("cargo::rustc-check-cfg=cfg(diffsol_step_limit)");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.join("../..");
    // A crate installed from crates.io is built in its consumer's workspace,
    // where only this package's own files are available.
    let in_workspace = workspace
        .join("crates/qspquest-modeling/build.rs")
        .is_file()
        && workspace.join("Cargo.lock").is_file();
    let (root, mut files, backend, step_limit) = if in_workspace {
        let (backend, step_limit) = locked_diffsol(&workspace);
        (
            workspace.clone(),
            workspace_sources(&workspace),
            backend,
            step_limit,
        )
    } else {
        let mut files = Vec::new();
        sources(&manifest.join("src"), &mut files);
        files.extend([manifest.join("Cargo.toml"), manifest.join("build.rs")]);
        (
            manifest.clone(),
            files,
            "crates-io:unlocked".to_owned(),
            false,
        )
    };
    println!("cargo:rustc-env=QSPQUEST_DIFFSOL_BACKEND={backend}");
    if step_limit {
        println!("cargo:rustc-cfg=diffsol_step_limit");
    }
    files.sort();
    let mut hash = Sha256::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = relative_source_name(&root, &path);
        let relative = relative.as_bytes();
        let data = fs::read(&path).expect("source bytes");
        hash.update((relative.len() as u64).to_be_bytes());
        hash.update(relative);
        hash.update((data.len() as u64).to_be_bytes());
        hash.update(data);
    }
    println!(
        "cargo:rustc-env=QSPQUEST_SOURCE_SHA256=sha256:{:x}",
        hash.finalize()
    );
    for key in ["TARGET", "PROFILE"] {
        println!("cargo:rustc-env=QSPQUEST_{key}={}", env::var(key).unwrap());
    }
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    println!(
        "cargo:rustc-env=QSPQUEST_RUSTFLAGS_SHA256=sha256:{:x}",
        Sha256::digest(
            env::var("CARGO_ENCODED_RUSTFLAGS")
                .unwrap_or_default()
                .as_bytes()
        )
    );
    let rustc = Command::new(env::var("RUSTC").unwrap())
        .arg("--version")
        .output()
        .expect("rustc version");
    assert!(rustc.status.success());
    println!(
        "cargo:rustc-env=QSPQUEST_RUSTC_VERSION={}",
        String::from_utf8(rustc.stdout).unwrap().trim()
    );
}
