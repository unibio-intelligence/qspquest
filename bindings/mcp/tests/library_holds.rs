//! A library in which one archive fails a load check keeps serving every other
//! model, and reports the withheld archive or capability by model and check.
use qspquest_mcp::{
    library::{hash, ArchiveChecks, Library},
    policy::Tier,
    server::Server,
};
use rmcp::{
    model::{CallToolRequestParams, ReadResourceRequestParams},
    service::RunningService,
    RoleClient, ServiceExt,
};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const FITTED: &str = "qspquest_diffdose_2026_idr_benchmark";
const FITTED_VERSION: &str = "0.3.1";
const OTHER: &str = "qspquest_diffdose_2026_bite_benchmark";

static ID: AtomicU64 = AtomicU64::new(0);

/// A private copy of the synthetic test library.
struct Copy(PathBuf);
impl Copy {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qspquest-library-holds-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        copy_dir(
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/library")),
            &root,
        );
        Self(root)
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
    fn json(&self, relative: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.path(relative)).unwrap()).unwrap()
    }
    fn write_json(&self, relative: &str, value: &Value) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(value).unwrap();
        std::fs::write(self.path(relative), &bytes).unwrap();
        bytes
    }
    fn archive_path(&self, id: &str) -> String {
        self.json("catalog.json")["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()["archive"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    /// Replace an archive and carry its new digest through the catalog and the
    /// availability registry, so that only the archive's own checks can fail.
    fn replace_archive(&self, id: &str, archive: &[u8]) {
        let relative = self.archive_path(id);
        std::fs::write(self.path(&relative), archive).unwrap();
        let digest = hash(archive);
        let mut registry = self.json("qspquest_models/availability.json");
        for record in registry["records"].as_object_mut().unwrap().values_mut() {
            if record["model_id"] == id {
                record["archive_sha256"] = json!(digest);
            }
        }
        let registry = self.write_json("qspquest_models/availability.json", &registry);
        let mut catalog = self.json("catalog.json");
        for model in catalog["models"].as_array_mut().unwrap() {
            if model["id"] == id {
                model["omex_sha256"] = json!(digest);
            }
        }
        catalog["availability_sha256"] = json!(hash(&registry));
        self.write_json("catalog.json", &catalog);
    }
    /// Change the engine target recorded in one fitting qualification record of
    /// the fit-ready model, re-indexing every digest inside the archive.
    fn alter_fitting_evidence(&self) {
        let relative = self.archive_path(FITTED);
        let mut members = unzip(&std::fs::read(self.path(&relative)).unwrap());
        let mut fitting: Value = serde_json::from_slice(&members["fitting.json"]).unwrap();
        let evidence = fitting["workflows"][0]["qualification"]["evidence"].clone();
        let path = evidence["path"].as_str().unwrap().to_owned();
        let mut record: Value = serde_json::from_slice(&members[&path]).unwrap();
        record["engine_identity"]["target"] = json!("");
        let record = serde_json::to_vec(&record).unwrap();
        fitting["workflows"][0]["qualification"]["evidence"]["sha256"] = json!(hash(&record));
        let fitting = serde_json::to_vec(&fitting).unwrap();
        let mut index: Value = serde_json::from_slice(&members["index.json"]).unwrap();
        index["members"][&path] = json!(hash(&record));
        index["members"]["fitting.json"] = json!(hash(&fitting));
        members.insert(path, record);
        members.insert("fitting.json".into(), fitting);
        members.insert("index.json".into(), serde_json::to_vec(&index).unwrap());
        self.replace_archive(FITTED, &zip(&members));
    }
    /// Corrupt one archive so that it no longer matches its catalog digest.
    fn corrupt_archive(&self, id: &str) {
        let path = self.path(&self.archive_path(id));
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(0);
        std::fs::write(path, bytes).unwrap();
    }
}
impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn unzip(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            (file.name().to_owned(), contents)
        })
        .collect()
}

fn zip(members: &std::collections::BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in members {
        writer
            .start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn connect(
    library: Library,
) -> (RunningService<RoleClient, ()>, tokio::task::JoinHandle<()>) {
    let (a, b) = tokio::io::duplex(4_000_000);
    let server = Server::new(library, false, Tier::Free, 2);
    let task = tokio::spawn(async move {
        server.serve(a).await.unwrap().waiting().await.unwrap();
    });
    (().serve(b).await.unwrap(), task)
}

async fn call(client: &RunningService<RoleClient, ()>, tool: &str, arguments: Value) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(tool.to_owned())
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    result.structured_content.unwrap_or_else(|| {
        serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap()
    })
}

async fn capabilities(client: &RunningService<RoleClient, ()>) -> Value {
    let resource = client
        .read_resource(ReadResourceRequestParams::new("qspquest://capabilities"))
        .await
        .unwrap();
    serde_json::from_str(match &resource.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text,
        _ => panic!("capabilities must be text"),
    })
    .unwrap()
}

/// The first stored preset of a model runs and returns at least one output.
async fn preset_runs(client: &RunningService<RoleClient, ()>, id: &str) {
    let card = call(client, "get_model", json!({"id": id})).await;
    let scenario = card["scenario_ids"][0].as_str().unwrap().to_owned();
    let run = call(
        client,
        "simulate",
        json!({"model":{"scenario":{"model_id":id,"scenario_id":scenario}}}),
    )
    .await;
    assert!(run.get("error").is_none(), "{id}: {run}");
    assert!(!run["outputs"].as_array().unwrap().is_empty(), "{id}");
}

#[tokio::test]
async fn failed_fitting_evidence_withholds_only_fitting_of_that_model() {
    let library = Copy::new();
    library.alter_fitting_evidence();

    let error = Library::directory_with(&library.0, ArchiveChecks::Strict)
        .err()
        .unwrap();
    assert!(
        error.starts_with(&format!(
            "Model {FITTED} {FITTED_VERSION} failed the fitting_evidence check: Workflow "
        )),
        "{error}"
    );
    assert!(error.contains("engine_identity.target"), "{error}");

    let loaded = Library::directory_with(&library.0, ArchiveChecks::Hold).unwrap();
    let holds = loaded.holds();
    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].model_id, FITTED);
    assert_eq!(holds[0].version, FITTED_VERSION);
    assert_eq!(holds[0].check, "fitting_evidence");
    assert!(holds[0].message.contains("engine_identity.target"));
    let (client, task) = connect(loaded).await;

    let search = call(&client, "search_models", json!({})).await;
    assert_eq!(search["archive_holds"][0]["model_id"], FITTED);
    let row = search["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == FITTED)
        .unwrap();
    assert_eq!(row["archive_hold"]["scope"], "fitting");
    assert_eq!(row["fit_ready"], false);
    assert_eq!(row["execution_available"], true);

    let card = call(&client, "get_model", json!({"id": FITTED})).await;
    assert_eq!(
        card["metadata"]["archive_hold"]["check"],
        "fitting_evidence"
    );
    assert_eq!(card["fitting"]["supported"], false);
    let availability = &card["metadata"]["availability"];
    assert_eq!(availability["fit_ready"], false);
    assert!(!availability["supported_operations"]
        .as_array()
        .unwrap()
        .contains(&json!("fit")));
    assert!(availability["blockers"]
        .as_array()
        .unwrap()
        .contains(&json!("archive_hold_fitting_evidence")));

    let caps = capabilities(&client).await;
    assert_eq!(caps["archive_holds"][0]["model_id"], FITTED);
    assert_eq!(caps["archive_holds"][0]["scope"], "fitting");

    let example: Value =
        serde_json::from_str(include_str!("../assets/fits/library-scalar.json")).unwrap();
    let refused = call(&client, "fit_scalar", example["arguments"].clone()).await;
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("Fitting is withheld"), "{refused}");
    assert!(message.contains("engine_identity.target"), "{refused}");

    preset_runs(&client, FITTED).await;
    preset_runs(&client, OTHER).await;
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn failed_archive_integrity_withholds_that_archive_and_serves_the_rest() {
    let library = Copy::new();
    library.corrupt_archive(OTHER);

    let error = Library::directory_with(&library.0, ArchiveChecks::Strict)
        .err()
        .unwrap();
    assert!(
        error.starts_with(&format!(
            "Model {OTHER} 0.1.0 failed the archive_integrity check: Archive checksum failed"
        )),
        "{error}"
    );

    let loaded = Library::directory_with(&library.0, ArchiveChecks::Hold).unwrap();
    let total = loaded.records.len();
    let (client, task) = connect(loaded).await;

    let search = call(&client, "search_models", json!({"limit": 100})).await;
    assert_eq!(search["total"], total);
    assert_eq!(search["archive_holds"].as_array().unwrap().len(), 1);
    let row = search["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == OTHER)
        .unwrap();
    assert_eq!(row["archive_hold"]["scope"], "archive");
    assert_eq!(row["archive_hold"]["check"], "archive_integrity");
    assert_eq!(row["execution_available"], false);

    let card = call(&client, "get_model", json!({"id": OTHER})).await;
    assert_eq!(card["execution_available"], false);
    assert!(card["metadata"]["execution_blocker"]
        .as_str()
        .unwrap()
        .contains("Archive checksum failed"));
    assert_eq!(card["metadata"]["archive_hold"]["model_id"], OTHER);

    let refused = call(
        &client,
        "simulate",
        json!({"model":{"library":{"model_id":OTHER}},"end":{"value":1,"unit":"h"}}),
    )
    .await;
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("archive_integrity"));

    let caps = capabilities(&client).await;
    assert_eq!(caps["archive_holds"][0]["scope"], "archive");

    preset_runs(&client, FITTED).await;
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn withheld_canonical_family_member_keeps_discovery_available() {
    const CANONICAL: &str = "qspquest_liu_2026_aav_mab_targetx_rat_brain_full_qsp";
    let library = Copy::new();
    library.corrupt_archive(CANONICAL);
    let loaded = Library::directory_with(&library.0, ArchiveChecks::Hold).unwrap();
    let (client, task) = connect(loaded).await;
    let search = call(&client, "search_models", json!({"limit": 100})).await;
    assert!(search.get("error").is_none(), "{search}");
    let row = search["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == CANONICAL)
        .unwrap();
    assert_eq!(row["archive_hold"]["scope"], "archive");
    assert_eq!(row["discovery"]["source_family"]["status"], "declared");
    preset_runs(&client, OTHER).await;
    client.cancel().await.unwrap();
    task.abort();
}
