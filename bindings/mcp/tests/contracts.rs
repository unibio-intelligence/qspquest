use qspquest_mcp::{
    cache::Cache,
    library::{hash, Library},
    policy,
};
use serde_json::json;
use std::path::PathBuf;
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("qspquest-mcp-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}
#[test]
fn local_directory_is_a_candidate_and_rejects_symlinks() {
    let path = scratch("directory");
    let file = path.join("oral.qsp");
    std::fs::write(&file, include_str!("../assets/templates/oral.qsp")).unwrap();
    let lib = Library::directory(&path).unwrap();
    assert_eq!(lib.records.len(), 1);
    assert_eq!(lib.records[0].metadata["candidate"], true);
    assert_eq!(
        lib.records[0].source_text(),
        include_str!("../assets/templates/oral.qsp")
    );
    let states = qspquest_mcp::library::display_states(&lib.records[0].model);
    assert_eq!(states.len(), 2);
    assert_eq!(states[0]["name"], "depot");
    assert_eq!(states[1]["name"], "central");
    assert!(states
        .iter()
        .all(|s| s["unit"] == "mg" && s["dosing"]["modes"] == json!(["bolus", "infusion"])));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&file, path.join("linked.qsp")).unwrap();
        assert!(Library::directory(&path).is_err());
    }
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn cache_is_bounded_and_never_advertises_an_unretained_result() {
    let cache = Cache::default();
    for n in 0..9 {
        assert!(cache.put(n.to_string(), json!({"n":n})));
    }
    assert!(cache.get("0").is_none());
    assert!(cache.get("8").is_some());
    assert!(!cache.put("large".into(), json!({"data":"x".repeat(4_000_001)})));
}
#[test]
fn refresh_rejects_bad_hash_and_links_without_moving_current_snapshot() {
    let path = scratch("refresh");
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_link(&mut header, "catalog.json", "/etc/passwd")
        .unwrap();
    let data = tar.into_inner().unwrap();
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&data).unwrap();
    let bytes = gzip.finish().unwrap();
    assert!(qspquest_mcp::refresh::unpack(&bytes, &hash(&bytes), &path).is_err());
    assert!(qspquest_mcp::refresh::unpack(&bytes, &"0".repeat(64), &path).is_err());
    assert!(std::fs::read_dir(&path).unwrap().next().is_none());
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn pinned_release_accepts_full_library_layout_and_reuses_verified_cache() {
    let path = scratch("release-cache");
    let mut tar = tar::Builder::new(Vec::new());
    let mut append = |name: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes).unwrap();
    };
    append("catalog.json", br#"{"models":[]}"#);
    // Public model releases include cards, notices and result files as well as OMEX.
    for n in 0..300 {
        append(
            &format!("models/synthetic/results/{n}.csv"),
            b"time,value\n0,1\n",
        );
    }
    let data = tar.into_inner().unwrap();
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&data).unwrap();
    let bytes = gzip.finish().unwrap();
    let sha = hash(&bytes);
    let library = qspquest_mcp::refresh::unpack(&bytes, &sha, &path).unwrap();
    assert!(library.records.is_empty());
    // No network access is needed after the pinned snapshot has been installed.
    let cached =
        qspquest_mcp::refresh::initial("https://invalid.example/library.tar.gz", &sha, &path)
            .await
            .unwrap();
    assert_eq!(cached.version, library.version);
    assert!(
        qspquest_mcp::refresh::initial("http://invalid.example/library.tar.gz", &sha, &path)
            .await
            .is_err()
    );
    std::fs::write(path.join(&sha).join("library/catalog.json"), b"invalid").unwrap();
    assert!(
        qspquest_mcp::refresh::initial("https://invalid.example/library.tar.gz", &sha, &path)
            .await
            .is_err()
    );
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn ambiguous_model_refs_and_unsupported_detail_do_not_silently_select_a_path() {
    assert!(serde_json::from_value::<qspquest_mcp::server::ModelRef>(
        json!({"source":"x","library":{"model_id":"x"}})
    )
    .is_err());
    let mut request = json!({"total_budgets":{"solver_callbacks":25_000_001}});
    assert!(policy::admit(&mut request, true).is_err());
}
#[test]
fn malformed_raw_requests_return_errors_before_friendly_overrides() {
    let model = json!({"time_unit":"h"});
    for bad in [json!(0), json!([]), json!("invalid"), json!(null)] {
        let args = json!({"request":{"schema":"qspquest.run/v0.1","regimen":bad},
            "end":{"value":1,"unit":"h"}});
        assert!(qspquest_mcp::core::requests::run(&args, None, &model)
            .unwrap_err()
            .contains("regimen must be an object"));
        for key in ["parameters", "initial_states"] {
            let mut args = json!({"request":{"schema":"qspquest.run/v0.1","regimen":{}},
                key:{"x":{"value":1,"unit":"1"}}});
            args["request"][key] = bad.clone();
            assert!(qspquest_mcp::core::requests::run(&args, None, &model)
                .unwrap_err()
                .contains(key));
        }
    }
}
#[cfg(feature = "sandbox")]
#[test]
fn sandbox_epoch_deadline_interrupts_independently_of_fuel() {
    // Minimal no-import module whose execute export loops forever. This tests
    // the epoch clock independently of model speed or the fuel ceiling.
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    fn section(bytes: &mut Vec<u8>, id: u8, data: Vec<u8>) {
        assert!(data.len() < 128);
        bytes.extend([id, data.len() as u8]);
        bytes.extend(data);
    }
    section(
        &mut bytes,
        1,
        vec![2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 1, 0x7e],
    );
    section(&mut bytes, 3, vec![2, 0, 1]);
    section(&mut bytes, 5, vec![1, 0, 1]);
    let mut exports = vec![3];
    for (name, kind, index) in [
        ("memory", 2, 0),
        ("qspquest_alloc", 0, 0),
        ("qspquest_execute", 0, 1),
    ] {
        exports.push(name.len() as u8);
        exports.extend(name.as_bytes());
        exports.extend([kind, index]);
    }
    section(&mut bytes, 7, exports);
    section(
        &mut bytes,
        10,
        vec![
            2, 4, 0, 0x41, 0, 0x0b, 9, 0, 0x03, 0x40, 0x0c, 0, 0x0b, 0x42, 0, 0x0b,
        ],
    );
    let sandbox = qspquest_mcp::sandbox::Sandbox::new(&bytes).unwrap();
    let begin = std::time::Instant::now();
    assert!(sandbox
        .limited("", "simulate", json!({}), 1, u64::MAX)
        .is_err());
    assert!(begin.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn friendly_fit_checks_engine_dimensions_before_expanding_tolerances() {
    let mut args = json!({"parameters_to_fit": vec![json!({"parameter":"p"}); 33]});
    let model = json!({"states":vec![json!({"name":"x","unit":"1"}); 513]});
    assert!(
        qspquest_mcp::core::requests::individual(&args, json!({}), &model)
            .unwrap_err()
            .contains("1..32")
    );
    args["parameters_to_fit"] = json!([{"parameter":"p"}]);
    assert!(
        qspquest_mcp::core::requests::individual(&args, json!({}), &model)
            .unwrap_err()
            .contains("1024 states")
    );
}
