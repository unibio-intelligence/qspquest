use qspquest_mcp::{
    deployment::{Inputs, Profile, Settings},
    library::Library,
    policy::Tier,
    server::Server,
};
use rmcp::{
    transport::{
        stdio,
        streamable_http_server::{
            session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        },
    },
    ServiceExt,
};
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut addr = None;
    let mut profile = std::env::var("QSPQUEST_MCP_PROFILE").ok();
    let mut output_dir = std::env::var_os("QSPQUEST_MCP_OUTPUT_DIR");
    let mut artifact_dir: Option<String> = None;
    let mut jobs_dir: Option<String> = None;
    let mut dir = std::env::var("QSPQUEST_LIBRARY_DIR").ok();
    let mut tier = std::env::var("QSPQUEST_MCP_TIER").unwrap_or("free".into());
    let mut concurrency = std::env::var("QSPQUEST_MCP_CONCURRENCY")
        .unwrap_or("2".into())
        .parse::<usize>()?;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--jobs-dir" => {
                jobs_dir = Some(args.next().ok_or("--jobs-dir requires PATH")?);
            }
            "--artifact-dir" => {
                artifact_dir = Some(args.next().ok_or("--artifact-dir requires PATH")?);
            }
            "--output-dir" => {
                output_dir = Some(args.next().ok_or("--output-dir requires PATH")?.into())
            }
            "--profile" => {
                profile = Some(
                    args.next()
                        .ok_or("--profile requires local|public|managed")?,
                )
            }
            "--http" => addr = Some(args.next().ok_or("--http requires ADDR")?),
            "--library-dir" => dir = Some(args.next().ok_or("--library-dir requires PATH")?),
            "--tier" => tier = args.next().ok_or("--tier requires VALUE")?,
            "--concurrency" => {
                concurrency = args.next().ok_or("--concurrency requires VALUE")?.parse()?
            }
            "--help" => {
                eprintln!(
                    "qspquest-mcp [--profile local|public|managed] [--http ADDR] [--library-dir PATH] [--output-dir PATH] [--artifact-dir PATH] [--jobs-dir PATH] [--tier free|pro|enterprise|admin] [--concurrency N]"
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown option {arg}").into()),
        }
    }
    if artifact_dir.is_some() && addr.is_some() {
        return Err("Result artifact storage is available only for local stdio servers".into());
    }
    if artifact_dir.as_ref().is_some_and(|p| p.is_empty()) {
        return Err("Result artifact directory cannot be empty".into());
    }
    if jobs_dir.is_some()
        && (addr.is_some()
            || artifact_dir.is_none()
            || jobs_dir.as_ref().is_some_and(|path| path.is_empty())
            || jobs_dir == artifact_dir
            || profile.as_deref().is_some_and(|p| p != "local"))
    {
        return Err("--jobs-dir requires local stdio, --artifact-dir and distinct nonempty private directories".into());
    }
    if !(1..=64).contains(&concurrency) {
        return Err("concurrency must be 1..64".into());
    }
    // Resolve once and bind this address, so diagnostics and ingress policy use
    // the same actual listener even when a hostname has several addresses.
    let bind_address = if let Some(address) = &addr {
        Some(
            tokio::net::lookup_host(address)
                .await?
                .next()
                .ok_or("HTTP address did not resolve")?,
        )
    } else {
        None
    };
    let managed_enabled = std::env::var("QSPQUEST_MANAGED_ENABLED").ok();
    let hosts = std::env::var("QSPQUEST_MCP_ALLOWED_HOSTS").ok();
    let origins = std::env::var("QSPQUEST_MCP_ALLOWED_ORIGINS").ok();
    let gateway = std::env::var("QSPQUEST_MCP_GATEWAY_ORIGIN").ok();
    let proxies = std::env::var("QSPQUEST_MCP_TRUSTED_PROXIES").ok();
    let deployment = Settings::parse(
        addr.is_some(),
        bind_address.is_none_or(|a| a.ip().is_loopback()),
        Inputs {
            requested_profile: profile.as_deref(),
            managed_enabled: managed_enabled.as_deref(),
            allowed_hosts: hosts.as_deref(),
            allowed_origins: origins.as_deref(),
            gateway_origin: gateway.as_deref(),
            trusted_proxies: proxies.as_deref(),
        },
    )?;
    #[cfg(not(feature = "managed"))]
    if deployment.profile == Profile::Managed {
        return Err("Managed profile requires a managed feature build".into());
    }
    #[cfg(feature = "managed")]
    let resolver = if deployment.profile == Profile::Managed {
        qspquest_mcp::jobs::HttpJobBackend::configured().map_err(|e| e.to_string())?;
        let resolver = Arc::new(qspquest_mcp::jobs::SessionResolver::configured()?);
        resolver.probe().await?;
        eprintln!("Managed session resolver readiness: reachable (credential-free probe)");
        Some(resolver)
    } else {
        None
    };
    deployment.diagnostics();
    let resolver_ready = deployment.profile == Profile::Managed;
    let lib = if let Some(dir) = dir {
        Library::directory(std::path::Path::new(&dir))?
    } else {
        let url = std::env::var("QSPQUEST_LIBRARY_RELEASE_URL").map_err(|_| {
            "Set QSPQUEST_LIBRARY_RELEASE_URL and QSPQUEST_LIBRARY_RELEASE_SHA256 to a published model-library archive, or pass --library-dir PATH"
        })?;
        let sha = std::env::var("QSPQUEST_LIBRARY_RELEASE_SHA256").map_err(|_| {
            "QSPQUEST_LIBRARY_RELEASE_SHA256 is required for a model-library download"
        })?;
        let cache = qspquest_mcp::refresh::cache_path()?;
        qspquest_mcp::refresh::initial(&url, &sha, &cache).await?
    };
    let server = Server::new(lib, addr.is_some(), Tier::parse(&tier)?, concurrency)
        .with_deployment(&deployment, resolver_ready);
    let server = if let Some(path) = output_dir {
        if path.is_empty() {
            return Err("CSV output directory cannot be empty".into());
        }
        server.with_csv_output(std::path::Path::new(&path))?
    } else {
        server
    };
    let server = if let Some(path) = artifact_dir {
        server.with_artifact_storage(std::path::Path::new(&path))?
    } else {
        server
    };
    #[cfg(feature = "sandbox")]
    let server = {
        let mut server = server;
        server.sandbox = Some(Arc::new(qspquest_mcp::sandbox::Sandbox::embedded()?));
        server
    };
    let server = if let Some(path) = jobs_dir {
        server.with_local_jobs(std::path::Path::new(&path))?
    } else {
        server
    };
    server.start_local_jobs()?;
    qspquest_mcp::refresh::start(server.library.clone());
    if let Some(addr) = addr {
        if std::env::var("QSPQUEST_MCP_PREWARM").as_deref() != Ok("0") {
            eprintln!("Prewarming at most eight verified interactive library models");
            server.prewarm().await?;
        }
        #[cfg(not(feature = "sandbox"))]
        eprintln!("Hosted source compute is disabled: compile with --features sandbox.");
        let mut config = StreamableHttpServerConfig::default();
        config.max_request_body_bytes = 1_000_000;
        config.legacy_session_mode = false;
        config.json_response = true;
        config.allowed_hosts = deployment.allowed_hosts;
        config = config
            .with_allowed_origins(deployment.allowed_origins)
            .enforce_origin_validation();
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(NeverSessionManager::default()),
            config,
        );
        let app = axum::Router::new()
            .route("/healthz", axum::routing::get(|| async { "ok" }))
            .nest_service("/mcp", service)
            .layer(axum::extract::DefaultBodyLimit::max(1_000_000));
        #[cfg(feature = "sandbox")]
        let app = app.route(
            "/assets/engine.wasm",
            axum::routing::get(|| async {
                (
                    [
                        (axum::http::header::CONTENT_TYPE, "application/wasm"),
                        (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                    ],
                    include_bytes!(concat!(env!("OUT_DIR"), "/sandbox.wasm")).as_slice(),
                )
            }),
        );
        #[cfg(feature = "widget")]
        let app = app
            .route(
                "/assets/js/engine-worker.mjs",
                axum::routing::get(|| async {
                    (
                        [
                            (axum::http::header::CONTENT_TYPE, "text/javascript"),
                            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                        ],
                        include_str!("../../wasm/js/engine-worker.mjs"),
                    )
                }),
            )
            .route(
                "/assets/pkg/engine/qspquest_wasm.js",
                axum::routing::get(|| async {
                    (
                        [
                            (axum::http::header::CONTENT_TYPE, "text/javascript"),
                            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                        ],
                        include_str!(concat!(env!("OUT_DIR"), "/browser-engine.mjs")),
                    )
                }),
            )
            .route(
                "/assets/pkg/engine/qspquest_wasm_bg.wasm",
                axum::routing::get(|| async {
                    (
                        [
                            (axum::http::header::CONTENT_TYPE, "application/wasm"),
                            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                        ],
                        include_bytes!(concat!(env!("OUT_DIR"), "/browser-engine.wasm")).as_slice(),
                    )
                }),
            );
        #[cfg(feature = "managed")]
        let app = if let Some(resolver) = resolver {
            app.layer(axum::middleware::from_fn_with_state(
                resolver,
                qspquest_mcp::jobs::authenticate,
            ))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(deployment.trusted_proxies),
                qspquest_mcp::deployment::proxy_ingress,
            ))
        } else {
            app
        };
        let listener = tokio::net::TcpListener::bind(bind_address.unwrap()).await?;
        eprintln!("QSPQuest MCP listening on {addr}/mcp");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    } else {
        server.serve(stdio()).await?.waiting().await?;
    }
    Ok(())
}
