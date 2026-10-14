#![cfg(feature = "managed")]
use qspquest_mcp::discovery::{Backend, ExecutionAssessment, ExecutionAvailability, ScenarioCount};
use qspquest_mcp::jobs::{tenant_slots, HttpJobBackend, JobBackend, Scope};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
fn price() -> Value {
    json!({"credit_cost":42,"billing_unit":"credits","pricing_model":"fixed_fixture","pricing_note":"Synthetic fixed operation"})
}
fn job() -> Value {
    json!({"id":"job-1","run_id":"run-1","destination_project_id":"project-1","state":"succeeded","pricing":price(),"staged_outputs":[]})
}
fn submission() -> Value {
    json!({"run":{"id":"run-1","destination_project_id":"project-1","capability_id":"pkpd.fit_data","capability_version":"1","state":"ready"},"jobs":[job()],"pricing":price(),"status_href":"/runs/run-1","workbench_href":"/workbench/run-1"})
}
#[tokio::test]
async fn governed_preview_digest_idempotency_session_and_free_boundary() {
    use axum::{
        extract::State,
        http::HeaderMap,
        routing::{get, post},
        Json,
    };
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    // Immutable synthetic bytes only; no native execution/scientific qualification.
    let blob = Arc::new(vec![b'x'; 4_800_001]);
    let blob_sha = qspquest_mcp::library::hash(&blob);
    let created = qspquest_mcp::managed_artifacts::clock().unwrap();
    let range_mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let byte_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let metadata_sha = blob_sha.clone();
    let download_blob = blob.clone();
    let download_sha = blob_sha.clone();
    let download_mode = range_mode.clone();
    let download_count = byte_requests.clone();
    let app = axum::Router::new()
        .route("/pkpd/models", get(|| async {
            Json(json!({"data":{"models":[
                {"id":"catalog-unknown","status":"ready"},
                {"id":"catalog-available","version":"1","execution_available":true,"scenario_count":3},
                {"id":"catalog-unavailable","execution_available":false},
                {"id":"catalog-locked","access":{"tier_locked":true}}
            ]}}))
        }))
        .route(
            "/runs/preview",
            post(
                |State(seen): State<Arc<Mutex<Vec<String>>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer test-session");
                    assert!(body.get("idempotency_key").is_none());
                    seen.lock().unwrap().push("preview".into());
                    Json(json!({"data":{"capability_id":"pkpd.fit_data","capability_version":"1","valid":true,"preview_digest":"a".repeat(64),"approval_required":false,"pricing":price(),"workbench_href":"/workbench"}}))
                },
            ),
        )
        .route(
            "/runs",
            post(
                |State(seen): State<Arc<Mutex<Vec<String>>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    assert_eq!(headers["idempotency-key"], "retry-key-1");
                    assert_eq!(body["expected_preview_digest"], "a".repeat(64));
                    assert!(body.get("idempotency_key").is_none());
                    seen.lock().unwrap().push("submit".into());
                    Json(json!({"data":submission()}))
                },
            ),
        )
        .route("/jobs/forbidden", get(|| async {
            (axum::http::StatusCode::FORBIDDEN, Json(json!({"private_data":"TENANT_PRIVATE_CANARY"})))
        }))
        .route(
            "/jobs/job-1",
            get(|| async {
                Json(json!({"data":job()}))
            }),
        )
        .route(
            "/artifacts/artifact-1",
            get(|| async { Json(json!({"data":{"id":"artifact-1","organization_id":"tenant-1","project_id":"project-1","run_id":"run-1","job_id":"job-1","kind":"population_fit","name":"Synthetic fit","media_type":"application/json","size_bytes":2,"checksum_hex":"b".repeat(64),"download_href":"/artifacts/artifact-1/download"}})) }),
        )
        .route("/artifacts/retained-1", get(move || {
            let digest = metadata_sha.clone();
            async move { Json(json!({"data":{"id":"retained-1","organization_id":"tenant-1","project_id":"project-1","run_id":"run-1","job_id":"job-1","kind":"result","name":"Synthetic byte fixture","media_type":"application/octet-stream","size_bytes":4_800_001,"checksum_hex":digest,"download_href":"/artifacts/retained-1/download",
                "result_retention":{"schema":"qspquest.platform-result-bytes/v1","immutable":true,"created_at_unix_seconds":created,"expires_at_unix_seconds":created+600,"retention_seconds":600,"result_schema":null},"unused_metadata":"x".repeat(160_000)}})) }
        }))
        .route("/artifacts/retained-1/download", get(move |headers: HeaderMap| {
            let bytes = download_blob.clone(); let digest = download_sha.clone();
            let mode = download_mode.load(std::sync::atomic::Ordering::Relaxed);
            download_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            async move {
                assert_eq!(headers["authorization"], "Bearer test-session");
                assert_eq!(headers["if-match"].to_str().unwrap(), format!("\"sha256-{digest}\""));
                assert_eq!(headers["accept-encoding"], "identity");
                let range = headers["range"].to_str().unwrap().strip_prefix("bytes=").unwrap();
                let (start,end)=range.split_once('-').unwrap();
                let start=start.parse::<usize>().unwrap(); let end=end.parse::<usize>().unwrap();
                assert!(end-start+1<=48_000 && end<bytes.len());
                let mut body=bytes[start..=end].to_vec();
                if mode==4 {body.pop();}
                if mode==5 {body.push(b'x');}
                axum::http::Response::builder()
                    .status(if mode==1 {200} else {206})
                    .header("content-type","application/octet-stream")
                    .header("etag",if mode==2 {"\"changed\"".into()} else {format!("\"sha256-{digest}\"")})
                    .header("content-range",if mode==3 {format!("bytes 1-{end}/{}",bytes.len())} else {format!("bytes {start}-{end}/{}",bytes.len())})
                    .body(axum::body::Body::from(body)).unwrap()
            }
        }))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("QSPQUEST_PLATFORM_API_URL", format!("http://{addr}"));
    let backend = HttpJobBackend::configured().unwrap();
    let mut scope = Scope {
        user_id: "principal-1".into(),
        tenant_id: "tenant-1".into(),
        tier: "free".into(),
        bearer: "test-session".into(),
    };
    let mut input = json!({"platform_request":{"capability_id":"pkpd.fit_data","destination_project_id":"project-1","inputs":[],"parameters":{}},"idempotency_key":"retry-key-1"});
    assert!(backend.submit(&scope, input.clone()).await.is_err());
    assert!(seen.lock().unwrap().is_empty());
    scope.tier = "pro".into();
    assert_eq!(
        backend.submit(&scope, input.clone()).await.unwrap_err()["error"]["code"],
        "preview_required"
    );
    assert!(seen.lock().unwrap().is_empty());
    let preview = backend
        .preview(&scope, input["platform_request"].clone())
        .await
        .unwrap();
    assert_eq!(preview["managed_preview"]["pricing"]["credit_cost"], 42);
    println!(
        "Synthetic preview: {} credits; no Run created.",
        preview["managed_preview"]["pricing"]["credit_cost"]
    );
    assert_eq!(*seen.lock().unwrap(), vec!["preview"]);
    input["platform_request"]["expected_preview_digest"] =
        preview["managed_preview"]["preview_digest"].clone();
    let submitted = backend.submit(&scope, input.clone()).await.unwrap();
    let retry = backend.submit(&scope, input).await.unwrap();
    assert_eq!(submitted["managed_submission"]["jobs"][0]["id"], "job-1");
    assert_eq!(submitted, retry);
    assert_eq!(*seen.lock().unwrap(), vec!["preview", "submit", "submit"]);
    assert_eq!(
        backend.get(&scope, "job-1").await.unwrap()["managed_job"]["state"],
        "succeeded"
    );
    assert_eq!(
        backend.artifact(&scope, "artifact-1").await.unwrap()["managed_artifact"]["id"],
        "artifact-1"
    );
    let legacy = backend.artifact(&scope, "artifact-1").await.unwrap();
    assert_eq!(legacy["artifact_retrieval"]["state"], "unavailable");
    assert!(legacy.get("full_result_resource").is_none());
    let retained = backend.artifact(&scope, "retained-1").await.unwrap();
    assert_eq!(
        retained["platform_response"]["omitted_for_response_budget"],
        true
    );
    let uri = retained["full_result_resource"].as_str().unwrap();
    assert_eq!(
        retained["artifact_retrieval"]["descriptor"]["bytes"],
        4_800_001
    );
    assert_eq!(
        backend.artifact_resource(&scope, uri).await.unwrap()["chunk_count"],
        101
    );
    let first = backend
        .artifact_resource(&scope, &format!("{uri}/chunks/0"))
        .await
        .unwrap();
    assert_eq!(first["bytes"], 48_000);
    assert_eq!(
        first["sha256"],
        qspquest_mcp::library::hash(&blob[..48_000])
    );
    let last = backend
        .artifact_resource(&scope, &format!("{uri}/chunks/100"))
        .await
        .unwrap();
    assert_eq!(last["data"], "eA==");
    assert_eq!(last["offset"], 4_800_000);
    assert_eq!(last["bytes"], 1);
    assert!(backend
        .artifact_resource(&scope, &uri.replace(&blob_sha, &"0".repeat(64)))
        .await
        .is_err());
    assert!(backend
        .artifact_resource(&scope, &format!("{uri}/chunks/101"))
        .await
        .is_err());
    for mode in 1..=5 {
        range_mode.store(mode, std::sync::atomic::Ordering::Relaxed);
        let failure = backend
            .artifact_resource(&scope, &format!("{uri}/chunks/0"))
            .await
            .unwrap_err();
        assert_eq!(failure["error"]["code"], "managed_artifact_range");
    }
    range_mode.store(0, std::sync::atomic::Ordering::Relaxed);
    let before = byte_requests.load(std::sync::atomic::Ordering::Relaxed);
    let mut denied = scope.clone();
    denied.tenant_id = "tenant-2".into();
    let failure = backend
        .artifact_resource(&denied, &format!("{uri}/chunks/0"))
        .await
        .unwrap_err();
    assert!(!failure.to_string().contains("Synthetic byte fixture"));
    denied = scope.clone();
    denied.tier = "free".into();
    assert!(backend
        .artifact_resource(&denied, &format!("{uri}/chunks/0"))
        .await
        .is_err());
    let slots = tenant_slots("tenant-1");
    let slot1 = slots.clone().try_acquire_owned().unwrap();
    let slot2 = slots.try_acquire_owned().unwrap();
    assert_eq!(
        backend.artifact_resource(&scope, uri).await.unwrap_err()["error"]["code"],
        "tenant_busy"
    );
    drop((slot1, slot2));
    assert_eq!(
        byte_requests.load(std::sync::atomic::Ordering::Relaxed),
        before
    );
    let catalog = qspquest_mcp::jobs::PlatformModelSource::configured().unwrap();
    let discovered = catalog
        .search_discovery(&scope, "", &json!({}))
        .await
        .unwrap();
    assert_eq!(discovered.len(), 3);
    assert_eq!(
        discovered[0].execution_available,
        ExecutionAvailability::Unassessed
    );
    assert_eq!(discovered[0].scenario_count, ScenarioCount::Unrecorded);
    assert_eq!(
        discovered[0].discovery.execution_assessment,
        ExecutionAssessment::RequiresGovernedPreview
    );
    assert_eq!(
        discovered[1].discovery.executable_backend,
        Some(Backend::GovernedJob)
    );
    assert_eq!(
        discovered[2].discovery.execution_assessment,
        ExecutionAssessment::Unavailable
    );
    let exact = catalog
        .search_discovery(
            &scope,
            "",
            &json!({"model_id":"catalog-available","version":"1"}),
        )
        .await
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert!(catalog
        .search_discovery(
            &scope,
            "",
            &json!({"model_id":"catalog-available","version":"missing"})
        )
        .await
        .unwrap()
        .is_empty());
    let mut free_scope = scope.clone();
    free_scope.tier = "free".into();
    assert!(catalog
        .search_discovery(&free_scope, "", &json!({}))
        .await
        .is_err());
    let rejected = backend.get(&scope, "forbidden").await.unwrap_err();
    assert_eq!(rejected["error"]["status"], 403);
    assert!(!rejected.to_string().contains("TENANT_PRIVATE_CANARY"));
    scope.tenant_id = "tenant-2".into();
    let cross_tenant = backend.artifact(&scope, "artifact-1").await.unwrap_err();
    assert_eq!(cross_tenant["error"]["code"], "platform_response");
    assert!(!cross_tenant.to_string().contains("Synthetic fit"));
    scope.tenant_id = "tenant-1".into();
    assert!(backend.get(&scope, "../../wrong").await.is_err());
    assert!(!serde_json::to_string(&scope)
        .unwrap()
        .contains("test-session"));
    let a = tenant_slots("tenant-1");
    let b = tenant_slots("tenant-1");
    let c = tenant_slots("tenant-2");
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
    let p = a.clone().try_acquire_owned().unwrap();
    let q = a.clone().try_acquire_owned().unwrap();
    assert!(b.try_acquire_owned().is_err());
    drop((p, q));
    assert_eq!(a.available_permits(), 2);
    serve.abort();
    std::env::remove_var("QSPQUEST_PLATFORM_API_URL");
}
