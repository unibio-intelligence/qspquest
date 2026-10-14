use qspquest_mcp::{
    export::{csv_path, OutputDirectory},
    library::Library,
    policy::Tier,
    server::Server,
};
use rmcp::{model::CallToolRequestParams, ServiceExt};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "qspquest-csv-test-{}-{}",
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
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn nested_exports_create_new_files_and_preserve_existing_bytes() {
    let scratch = Scratch::new();
    let output = OutputDirectory::open(&scratch.0.join("output")).unwrap();
    let path = output
        .write_csv("study/nested/run.csv", b"time_h,side,cp\n0,post,1\n")
        .unwrap();
    assert!(path.is_absolute());
    assert_eq!(std::fs::read(&path).unwrap(), b"time_h,side,cp\n0,post,1\n");
    assert!(output
        .write_csv("study/nested/run.csv", b"replacement")
        .is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"time_h,side,cp\n0,post,1\n");
}
#[test]
fn bundle_reserves_both_names_and_preserves_sidecar_collision() {
    let scratch = Scratch::new();
    let root = scratch.0.join("output");
    let output = OutputDirectory::open(&root).unwrap();
    std::fs::write(root.join("run.csv.manifest.json"), b"existing metadata").unwrap();
    assert!(output
        .write_bundle("run.csv", b"table", b"replace")
        .is_err());
    assert!(!root.join("run.csv").exists());
    assert_eq!(
        std::fs::read(root.join("run.csv.manifest.json")).unwrap(),
        b"existing metadata"
    );
    let (csv, meta) = output
        .write_bundle("study/new.csv", b"table", b"manifest")
        .unwrap();
    assert_eq!(std::fs::read(csv).unwrap(), b"table");
    assert_eq!(std::fs::read(meta).unwrap(), b"manifest");
    assert!(output
        .write_bundle("study/new.csv", b"replace", b"replace")
        .is_err());
}
#[test]
fn simultaneous_bundles_have_one_complete_winner() {
    let scratch = Scratch::new();
    let output = OutputDirectory::open(&scratch.0.join("output")).unwrap();
    let barrier = std::sync::Barrier::new(4);
    let winners = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let (output, barrier) = (&output, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    let payload = i.to_string();
                    output
                        .write_bundle("run.csv", payload.as_bytes(), payload.as_bytes())
                        .is_ok()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count()
    });
    assert_eq!(winners, 1);
    assert_eq!(
        std::fs::read(scratch.0.join("output/run.csv")).unwrap(),
        std::fs::read(scratch.0.join("output/run.csv.manifest.json")).unwrap()
    );
}
#[test]
fn portable_paths_refuse_traversal_aliases_and_device_names() {
    for path in [
        "",
        "/run.csv",
        "../run.csv",
        "study/../run.csv",
        "study//run.csv",
        "./run.csv",
        "study\\run.csv",
        "run.csv:stream",
        "CON.csv",
        "study/NUL.csv",
        "COM1.csv",
        "LPT².csv",
        "study./run.csv",
        "study /run.csv",
        "run.txt",
    ] {
        assert!(csv_path(path).is_err(), "{path}");
    }
    assert!(csv_path("study/actual_run-1.csv").is_ok());
}
#[test]
fn simultaneous_exports_do_not_overwrite_the_winner() {
    let scratch = Scratch::new();
    let output = OutputDirectory::open(&scratch.0.join("output")).unwrap();
    let barrier = std::sync::Barrier::new(4);
    let count = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|n| {
                let (output, barrier) = (&output, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    output
                        .write_csv("run.csv", format!("{n}").as_bytes())
                        .is_ok()
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|success| *success)
            .count()
    });
    assert_eq!(count, 1);
    assert_eq!(
        std::fs::read(scratch.0.join("output/run.csv"))
            .unwrap()
            .len(),
        1
    );
}
#[cfg(any(unix, windows))]
#[test]
fn file_and_parent_links_cannot_redirect_exports() {
    let scratch = Scratch::new();
    let root = scratch.0.join("output");
    let output = OutputDirectory::open(&root).unwrap();
    let outside = scratch.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let destination = outside.join("protected.csv");
    std::fs::write(&destination, b"keep").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&destination, root.join("file.csv")).unwrap();
        symlink(&outside, root.join("directory")).unwrap();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        symlink_file(&destination, root.join("file.csv")).unwrap();
        symlink_dir(&outside, root.join("directory")).unwrap();
    }
    assert!(output.write_csv("file.csv", b"replace").is_err());
    assert!(output
        .write_csv("directory/protected.csv", b"replace")
        .is_err());
    assert!(output.write_csv("directory/new.csv", b"escape").is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
    assert!(!outside.join("new.csv").exists());
    #[cfg(unix)]
    std::os::unix::fs::symlink(&destination, root.join("bundle.csv.manifest.json")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&destination, root.join("bundle.csv.manifest.json"))
        .unwrap();
    assert!(output
        .write_bundle("bundle.csv", b"table", b"replace")
        .is_err());
    assert!(!root.join("bundle.csv").exists());
    assert_eq!(
        std::fs::read(outside.join("protected.csv")).unwrap(),
        b"keep"
    );
}
#[cfg(unix)]
#[test]
fn replacing_root_path_does_not_replace_open_directory_anchor() {
    use std::os::unix::fs::symlink;
    let scratch = Scratch::new();
    let root = scratch.0.join("output");
    let output = OutputDirectory::open(&root).unwrap();
    let original = scratch.0.join("original");
    std::fs::rename(&root, &original).unwrap();
    let outside = scratch.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, &root).unwrap();
    output
        .write_bundle("run.csv", b"anchored", b"anchored metadata")
        .unwrap();
    assert_eq!(
        std::fs::read(original.join("run.csv.manifest.json")).unwrap(),
        b"anchored metadata"
    );
    assert_eq!(
        std::fs::read(original.join("run.csv")).unwrap(),
        b"anchored"
    );
    assert!(!outside.join("run.csv").exists());
}

#[cfg(windows)]
#[test]
fn root_cannot_be_renamed_until_directory_anchors_close() {
    let scratch = Scratch::new();
    let root = scratch.0.join("output");
    let moved = scratch.0.join("moved");
    let output = OutputDirectory::open(&root).unwrap();
    assert!(std::fs::rename(&root, &moved).is_err());
    output
        .write_bundle("safe.csv", b"data", b"metadata")
        .unwrap();
    assert!(root.join("safe.csv.manifest.json").is_file());
    drop(output);
    std::fs::rename(&root, &moved).unwrap();
    assert_eq!(std::fs::read(moved.join("safe.csv")).unwrap(), b"data");
}
#[tokio::test]
async fn real_mcp_client_observes_export_boundary_for_simulation_and_comparison() {
    let scratch = Scratch::new();
    let root = scratch.0.join("output");
    let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2)
        .with_csv_output(&root)
        .unwrap();
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let model = json!({"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}});
    for (name, args, ok) in [
        (
            "simulate",
            json!({"model":model,"save_csv":"nested/run.csv"}),
            true,
        ),
        (
            "simulate",
            json!({"model":model,"save_csv":"nested/run.csv"}),
            false,
        ),
        (
            "simulate",
            json!({"model":model,"save_csv":"../outside.csv"}),
            false,
        ),
        (
            "compare_regimens",
            json!({"model":model,"regimens":[{"name":"first","doses":[]}],"save_csv":"nested/compare.csv"}),
            true,
        ),
        (
            "compare_regimens",
            json!({"model":model,"regimens":[{"name":"first","doses":[]}],"save_csv":"nested/compare.csv"}),
            false,
        ),
    ] {
        let response = client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(response.is_error == Some(true), !ok, "{name}: {response:?}");
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("nested/run.csv.manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["schema"], qspquest_mcp::table::SCHEMA);
    assert_eq!(manifest["qualification"]["qualified"], false);
    assert!(manifest["inputs"]["effective_request"].is_object());
    assert!(manifest["model"]["provenance"]["source"].is_string());
    assert!(root
        .join("nested/compare_first.csv.manifest.json")
        .is_file());
    let csv = std::fs::read_to_string(root.join("nested/run.csv")).unwrap();
    assert!(csv.starts_with("time_"));
    assert!(csv.contains(",side,"));
    assert!(root.join("nested/compare_first.csv").is_file());
    assert!(!scratch.0.join("outside.csv").exists());
    client.cancel().await.unwrap();
    task.abort();
}
#[tokio::test]
async fn hosted_and_unconfigured_servers_refuse_exports_before_model_execution() {
    let scratch = Scratch::new();
    assert!(
        Server::new(Library::embedded().unwrap(), true, Tier::Free, 2)
            .with_csv_output(&scratch.0.join("output"))
            .is_err()
    );
    for hosted in [false, true] {
        let server = Server::new(Library::embedded().unwrap(), hosted, Tier::Free, 2);
        let (a, b) = tokio::io::duplex(1_000_000);
        let task =
            tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
        let client = ().serve(b).await.unwrap();
        let args =
            json!({"model":{"source":"deliberately invalid model syntax"},"save_csv":"run.csv"});
        let response = client
            .call_tool(
                CallToolRequestParams::new("simulate")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(response.is_error, Some(true));
        let encoded = serde_json::to_value(&response).unwrap();
        let value: Value = response.structured_content.unwrap_or_else(|| {
            serde_json::from_str(encoded["content"][0]["text"].as_str().unwrap()).unwrap()
        });
        assert_eq!(value["error"]["code"], "save_csv");
        client.cancel().await.unwrap();
        task.abort();
    }
}
