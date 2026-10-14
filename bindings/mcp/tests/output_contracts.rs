//! Consume status from structuredContent; prose is never an input to these checks.
use qspquest_mcp::{
    contract::{CapabilityDocument, Contract, Outcome, ToolOutput},
    library::Library,
    policy::Tier,
    server::Server,
};
use rmcp::{
    model::{CallToolRequestParams, ReadResourceRequestParams},
    ServiceExt,
};
use serde_json::{json, Value};

#[tokio::test]
async fn discovery_schemas_capabilities_and_failure_variants_use_structured_content() {
    let library = Library::embedded().unwrap();
    let expected_library = library.version.clone();
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move {
        Server::new(library, false, Tier::Free, 3)
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(b).await.unwrap();
    let tools = client.list_tools(Default::default()).await.unwrap().tools;
    assert!(tools
        .iter()
        .all(|tool| tool.output_schema.is_some() && tool.annotations.is_some()));
    let response = client
        .read_resource(ReadResourceRequestParams::new("qspquest://capabilities"))
        .await
        .unwrap();
    let value: Value = serde_json::to_value(response).unwrap();
    let caps: CapabilityDocument =
        serde_json::from_str(value["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        caps.tools,
        tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(caps.library_version, expected_library);
    assert_eq!(caps.limits.compute_slots, 3);
    assert!(!caps.deployment.csv_export_enabled && caps.deployment.user_source_execution);
    assert_eq!(caps.features.analysis, cfg!(feature = "analysis"));
    assert_eq!(caps.features.managed, cfg!(feature = "managed"));
    assert_eq!(caps.features.sandbox, cfg!(feature = "sandbox"));
    assert!(matches!(
        caps.current_engine_qualification,
        qspquest_mcp::contract::ArchiveQualification::NotAssessed
    ));
    for tool in &tools {
        let annotations = tool.annotations.as_ref().unwrap();
        let write = matches!(
            tool.name.as_ref(),
            "simulate" | "compare_regimens" | "cancel_local_job" | "resume_local_job"
        ) || tool.name.starts_with("submit_");
        assert_eq!(
            annotations.read_only_hint,
            Some(!write),
            "tool {}",
            tool.name
        );
        assert_eq!(annotations.destructive_hint, Some(false));
    }
    for (name, arguments, status) in [
        ("search_models", json!({"query":"turnover"}), "success"),
        ("scan_parameters", json!({}), "request_failure"),
        (
            "simulate",
            json!({"model":{"source":"x", "library":{"model_id":"x"}}}),
            "request_failure",
        ),
        ("get_model", json!({"id":"missing"}), "request_failure"),
        (
            "model_template",
            json!({"kind":"oral", "options":{"unsupported":true}}),
            "unsupported",
        ),
        (
            "simulate",
            json!({"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}},"solver":"diffsol_rosenbrock23"}),
            "unsupported",
        ),
        (
            "simulate",
            json!({"model":{"source":"model synthetic_output_domain\ntime_unit = h\nparameters\n  k = 1 [1/h]\nstates\n  x = 0 [1]\ndynamics\n  d/dt(x) = (-k * x)\noutputs\n  inverse = (1 / x) [1]\nend\n"},"end":{"value":1,"unit":"h"}}),
            "scientific_failure",
        ),
        (
            "summarize_profile",
            json!({"times":[0],"values":[1]}),
            "request_failure",
        ),
    ] {
        let response = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let content = response
            .structured_content
            .expect("typed content is required even on failure");
        assert_eq!(content["contract"]["status"], status);
        let parsed: ToolOutput = serde_json::from_value(content).unwrap();
        assert_eq!(parsed.contract.library_version, expected_library);
        assert_eq!(parsed.library_version, expected_library);
        if status == "success" {
            assert!(matches!(parsed.contract.outcome, Outcome::Success));
        }
    }
    assert!(
        client
            .call_tool(CallToolRequestParams::new("missing_tool"))
            .await
            .is_err(),
        "Unknown tools remain protocol errors rather than scientific tool results"
    );
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn hosted_capabilities_and_heavy_model_routing_do_not_claim_a_result() {
    let server = Server::new(Library::embedded().unwrap(), true, Tier::Free, 2);
    let caps = server.capabilities();
    assert_eq!(caps["deployment"]["mode"], "hosted");
    assert_eq!(caps["deployment"]["csv_export_enabled"], false);
    assert_eq!(caps["deployment"]["user_source_execution"], false);
    assert!(caps["limits"]["local_result_entries"].is_null());
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let response = client.call_tool(CallToolRequestParams::new("simulate").with_arguments(json!({"model":{"library":{"model_id":"qspquest_liu_2026_aav_mab_targetx_rat_brain_full_qsp"}}}).as_object().unwrap().clone())).await.unwrap();
    let value = response.structured_content.unwrap();
    let contract: Contract = serde_json::from_value(value["contract"].clone()).unwrap();
    match contract.outcome {
        Outcome::AsyncRequired { route, .. } => {
            assert_eq!(route.tool, "submit_simulation");
            assert!(route.requires_verified_session && route.requires_platform_qualification);
        }
        status => panic!("wrong routing status: {status:?}"),
    }
    assert!(value.get("outputs").is_none());
    assert_eq!(response.is_error, Some(true));
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn scalar_fit_iteration_limit_retains_diagnostics_without_claiming_success() {
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move {
        Server::new(Library::embedded().unwrap(), false, Tier::Free, 2)
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(b).await.unwrap();
    let example: Value = serde_json::from_str(include_str!("../assets/fits/scalar.json")).unwrap();
    let mut arguments = example["arguments"].clone();
    arguments["max_iterations"] = json!(1);
    let response = client
        .call_tool(
            CallToolRequestParams::new("fit_scalar")
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(
        response.is_error,
        Some(true),
        "Diagnostic estimates preserve the existing protocol behavior"
    );
    let value = response.structured_content.unwrap();
    assert_eq!(value["status"], "iteration_limit");
    assert_eq!(value["scientific_status"]["converged"], false);
    assert_eq!(value["contract"]["status"], "scientific_failure");
    assert!(value["parameters"]["cl"]["value"].is_number());
    let _: ToolOutput = serde_json::from_value(value).unwrap();
    client.cancel().await.unwrap();
    task.abort();
}
