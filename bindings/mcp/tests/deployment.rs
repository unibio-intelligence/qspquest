#![cfg(feature = "managed")]
//! Complete synthetic deployment check against the production executable.
use axum::{
    http::{HeaderMap, StatusCode},
    routing::get,
    Json,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
struct ServerProcess(Child);
impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct FixtureDirectory(PathBuf);
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_qspquest-mcp"));
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("QSPQUEST_")) {
        command.env_remove(key);
    }
    command.env("QSPQUEST_MCP_PREWARM", "0");
    command
}
fn refuses(args: &[&str], env: &[(&str, &str)], message: &str) {
    let result = command()
        .args(args)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains(message), "{error}");
}
async fn start(
    profile: &str,
    library: &FixtureDirectory,
    platform: &str,
    peer: Option<&str>,
) -> (ServerProcess, String) {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let mut command = command();
    command
        .args([
            "--profile",
            profile,
            "--http",
            &address.to_string(),
            "--library-dir",
        ])
        .arg(&library.0)
        .env("QSPQUEST_MCP_ALLOWED_ORIGINS", "https://app.example.test")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if profile == "managed" {
        command.env("QSPQUEST_PLATFORM_API_URL", platform).env(
            "QSPQUEST_PLATFORM_SESSION_URL",
            format!("{platform}/session"),
        );
        if let Some(peer) = peer {
            command.env("QSPQUEST_MCP_TRUSTED_PROXIES", peer);
        }
    }
    let mut child = ServerProcess(command.spawn().unwrap());
    let base = format!("http://{address}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Server exited before readiness"
        );
        if client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .is_ok_and(|v| v.status().is_success())
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Server readiness timeout"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    (child, base)
}
async fn rpc(
    client: &reqwest::Client,
    base: &str,
    method: &str,
    params: Value,
    token: Option<&str>,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut request = client
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    request.send().await.unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_profiles_session_origin_and_actual_peer_boundary() {
    refuses(&["--profile", "managed"], &[], "require --http");
    refuses(
        &["--profile", "managed", "--http", "0.0.0.0:0"],
        &[],
        "Non-loopback managed HTTP requires",
    );
    refuses(
        &["--profile", "public", "--http", "127.0.0.1:0"],
        &[("QSPQUEST_MANAGED_ENABLED", "1")],
        "conflicts",
    );
    refuses(
        &["--profile", "public", "--http", "127.0.0.1:0"],
        &[("QSPQUEST_MCP_ALLOWED_ORIGINS", "null")],
        "Endpoint must",
    );
    refuses(
        &["--profile", "managed", "--http", "127.0.0.1:0"],
        &[],
        "QSPQUEST_PLATFORM_API_URL",
    );
    refuses(
        &["--profile", "managed", "--http", "127.0.0.1:0"],
        &[("QSPQUEST_PLATFORM_API_URL", "http://127.0.0.1:7")],
        "QSPQUEST_PLATFORM_SESSION_URL",
    );
    let app = axum::Router::new().route("/session", get(|headers: HeaderMap| async move {
        if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer synthetic-session") {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error":"synthetic resolver rejects wrong audience or token"})));
        }
        (StatusCode::OK, Json(json!({"data":{"principal":{"id":"principal-1","organization_id":"tenant-1"},"usage_plan":"pro"}})))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let platform = format!("http://{}", listener.local_addr().unwrap());
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    refuses(
        &["--profile", "managed", "--http", "127.0.0.1:0"],
        &[
            ("QSPQUEST_PLATFORM_API_URL", &platform),
            (
                "QSPQUEST_PLATFORM_SESSION_URL",
                &format!("{platform}/missing"),
            ),
        ],
        "readiness failed",
    );
    let directory = FixtureDirectory(std::env::temp_dir().join(format!(
            "qspquest-deployment-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::write(
        directory.0.join("oral.qsp"),
        include_str!("../assets/templates/oral.qsp"),
    )
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let (_public, public) = start("public", &directory, &platform, None).await;
    assert!(rpc(&client, &public, "tools/list", json!({}), None, &[])
        .await
        .status()
        .is_success());
    for origin in [
        "null",
        "https://wrong.example.test",
        "https://app.example.test:444",
    ] {
        assert_eq!(
            rpc(
                &client,
                &public,
                "tools/list",
                json!({}),
                None,
                &[("Origin", origin)]
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert!(rpc(
        &client,
        &public,
        "tools/list",
        json!({}),
        None,
        &[("Origin", "https://app.example.test")]
    )
    .await
    .status()
    .is_success());
    let (_managed, managed) = start("managed", &directory, &platform, None).await;
    for token in [None, Some("wrong-audience")] {
        assert_eq!(
            rpc(
                &client,
                &managed,
                "tools/list",
                json!({}),
                token,
                &[("X-Tier", "admin"), ("X-Tenant-ID", "tenant-1")]
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let caps: Value = rpc(
        &client,
        &managed,
        "resources/read",
        json!({"uri":"qspquest://capabilities"}),
        Some("synthetic-session"),
        &[],
    )
    .await
    .json()
    .await
    .unwrap();
    let caps: Value =
        serde_json::from_str(caps["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(caps["deployment"]["profile"], "managed");
    assert_eq!(caps["deployment"]["verified_session_resolver_ready"], true);
    assert_eq!(caps["deployment"]["proxy_peer_restriction"], true);
    let (_restricted, restricted) =
        start("managed", &directory, &platform, Some("127.0.0.2")).await;
    let denied = rpc(
        &client,
        &restricted,
        "tools/list",
        json!({}),
        Some("synthetic-session"),
        &[
            ("X-Forwarded-For", "127.0.0.2"),
            ("Forwarded", "for=127.0.0.2;proto=https"),
        ],
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        denied.json::<Value>().await.unwrap()["error"]["code"],
        "ingress_denied"
    );
    println!("Synthetic profiles: public discovery, exact Origins, resolver rejection, readiness failure and actual peer enforcement passed.");
    fixture.abort();
}
