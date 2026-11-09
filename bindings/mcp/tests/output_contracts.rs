//! Consume status from structuredContent; prose is never an input to these checks.
mod common;
use qspquest_mcp::{
    contract::{CapabilityDocument, Contract, Outcome, ToolOutput},
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
    let library = common::library();
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
            json!({"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}},"solver":"diffsol_rodas5p"}),
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
    let server = Server::new(common::library(), true, Tier::Free, 2);
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
        Server::new(common::library(), false, Tier::Free, 2)
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

#[cfg(feature = "analysis")]
fn visual_predictive_check() -> Value {
    let run: Value = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-one-compartment.json"
    ))
    .unwrap();
    // Synthetic subjects scattered about cp(t) = 2 exp(-0.1 t) mg/L.
    let scale = [0.9, 1.05, 1.2, 0.8];
    let subjects: Vec<Value> = scale
        .iter()
        .enumerate()
        .map(|(id, s)| {
            let observations: Vec<Value> = [1.0f64, 2.0, 4.0, 8.0, 12.0]
                .iter()
                .enumerate()
                .map(|(i, t)| json!({"row":i+1,"output":"cp","value":{"value":s*2.0*(-0.1*t).exp(),"unit":"mg/L"}}))
                .collect();
            json!({"id":id,"run":run,"observations":observations})
        })
        .collect();
    json!({"schema":"qspquest.analysis/v0.2","operation":"visual_predictive_check","arguments":{
        "subjects":subjects,"parameters":[{"name":"cl","value":0.3,"unit":"L/h"}],
        "omega":[[0.04]],"error":{"proportional":0.1},"replicates":5,"seed":11,
        "max_simulations":24}})
}

#[cfg(feature = "analysis")]
#[tokio::test]
async fn native_analysis_tools_return_typed_contracts_through_a_client() {
    let (a, b) = tokio::io::duplex(1_000_000);
    let task = tokio::spawn(async move {
        Server::new(common::library(), false, Tier::Free, 2)
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(b).await.unwrap();
    // tools/list advertises a compact schema; the complete one is a resource.
    let full_schema = |tool: &'static str| {
        let client = &client;
        async move {
            let response = client
                .read_resource(ReadResourceRequestParams::new(format!(
                    "qspquest://schema/tools/{tool}"
                )))
                .await
                .unwrap();
            let mut text = serde_json::to_value(response).unwrap()["contents"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned();
            let manifest: Value = serde_json::from_str(&text).unwrap();
            if manifest["kind"] == "utf8_chunk_manifest" {
                text.clear();
                for uri in manifest["chunks"].as_array().unwrap() {
                    let response = client
                        .read_resource(ReadResourceRequestParams::new(uri.as_str().unwrap()))
                        .await
                        .unwrap();
                    text.push_str(
                        serde_json::to_value(response).unwrap()["contents"][0]["text"]
                            .as_str()
                            .unwrap(),
                    );
                }
            }
            serde_json::from_str::<Value>(&text).unwrap()
        }
    };
    let tools = client.list_tools(Default::default()).await.unwrap().tools;
    assert!(tools.iter().any(|t| t.name == "analyze"));
    let schema = full_schema("analyze").await;
    let payload = &schema["allOf"][0]["then"]["properties"];
    for field in [
        "analysis",
        "analysis_operation",
        "analysis_status",
        "effective_request",
    ] {
        assert!(payload[field].is_object(), "{field}");
    }
    let schema = full_schema("compare_regimens").await;
    assert!(schema["allOf"][0]["then"]["properties"]["population"].is_object());
    let call = |name: &str, arguments: Value| {
        CallToolRequestParams::new(name.to_owned())
            .with_arguments(arguments.as_object().unwrap().clone())
    };
    let source = include_str!("../../../conformance/models/synthetic-one-compartment.qsp");
    let mut arguments = json!({"model":{"source":source},"request":visual_predictive_check()});
    let plan = client
        .call_tool(call(
            "plan_run",
            json!({"operation":"analyze","arguments":arguments}),
        ))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(plan["contract"]["status"], "success", "{plan}");
    let prepared = &plan["plan"]["native_preparation"]["plan"];
    assert_eq!(prepared["required_simulations"], 24);
    assert_eq!(prepared["simulations"], 0);
    assert_eq!(prepared["stochastic"], true);
    arguments["expected_plan_hash"] = plan["plan_hash"].clone();
    let content = client
        .call_tool(call("analyze", arguments))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(content["contract"]["status"], "success", "{content}");
    let parsed: ToolOutput = serde_json::from_value(content.clone()).unwrap();
    assert_eq!(parsed.identity, content["analysis"]["identity"]);
    assert_eq!(content["analysis_operation"], "visual_predictive_check");
    assert_eq!(content["plan_hash"], plan["plan_hash"]);
    assert_eq!(content["analysis"]["replicates"], 5);
    // A deterministic comparison keeps its shape; a population block replaces it.
    let base = json!({"model":{"source":source},"output":"cp","end":{"value":12,"unit":"h"},
        "regimens":[{"name":"low","doses":[{"target":"central","at":{"value":0,"unit":"h"},"amount":{"value":3,"unit":"mg"}}]},
                    {"name":"high","doses":[{"target":"central","at":{"value":0,"unit":"h"},"amount":{"value":6,"unit":"mg"}}]}]});
    let deterministic = client
        .call_tool(call("compare_regimens", base.clone()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        deterministic["contract"]["status"], "success",
        "{deterministic}"
    );
    assert_eq!(deterministic["regimens"].as_array().unwrap().len(), 2);
    assert!(deterministic.get("population").is_none());
    // One operation: the deterministic form is the native comparison with one subject.
    let native = &deterministic["comparison"];
    assert_eq!(native["schema"], "qspquest.regimen-comparison/v0.2");
    assert_eq!(native["variability"], "none");
    assert_eq!(native["simulations"], 0);
    let low = native["regimens"]["low"]["metric_percentiles"]["auc"][0]
        .as_f64()
        .unwrap();
    let difference = native["regimens"]["high"]["paired_differences_vs_reference"]["auc"][0]
        .as_f64()
        .unwrap();
    assert!((difference - low).abs() <= 1e-9 * low, "{low} {difference}");
    assert_eq!(deterministic["comparison_summary"]["variability"], "none");
    let mut population = base;
    population["population"] = json!({"parameters":[{"name":"cl","value":0.3,"unit":"L/h"}],
        "omega":[[0.09]],"subjects":10,"max_simulations":20,"threshold":0.5});
    let paired = client
        .call_tool(call("compare_regimens", population))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(paired["contract"]["status"], "success", "{paired}");
    serde_json::from_value::<ToolOutput>(paired.clone()).unwrap();
    let record = &paired["population"]["analysis"];
    assert_eq!(record["schema"], "qspquest.regimen-comparison/v0.2");
    assert_eq!(record["reference"], "low");
    assert!(record["regimens"]["high"]["metric_percentiles"]["time_above_threshold"].is_array());
    assert_eq!(paired["comparison_summary"]["subjects"], 10);
    client.cancel().await.unwrap();
    task.abort();
}
