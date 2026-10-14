use qspquest_mcp::{
    core::{compute, requests, summary},
    library::Library,
    policy::{self, Tier},
    server::Server,
};
use rmcp::{
    model::{CallToolRequestParams, ReadResourceRequestParams},
    ServiceExt,
};
use serde_json::{json, Value};
#[tokio::test]
async fn tools_through_real_mcp_client() {
    let (a, b) = tokio::io::duplex(1_000_000);
    let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let info = client.peer_info().unwrap();
    let implementation = info.server_info.as_ref().unwrap();
    assert_eq!(implementation.name, "qspquest-mcp");
    assert_eq!(implementation.version, env!("CARGO_PKG_VERSION"));
    let tools = client.list_tools(Default::default()).await.unwrap();
    assert_eq!(
        tools.tools.len(),
        20 + if cfg!(feature = "analysis") { 2 } else { 0 }
            + if cfg!(feature = "managed") { 7 } else { 0 }
    );
    // Validate schema-valued positions, preserving boolean keywords such as
    // additionalProperties. Claude's tool parser requires object subschemas.
    fn check_schema(schema: &Value) {
        assert!(schema.is_object(), "non-object schema: {schema}");
        for key in ["properties", "$defs", "definitions", "patternProperties"] {
            if let Some(values) = schema.get(key).and_then(Value::as_object) {
                for child in values.values() {
                    check_schema(child);
                }
            }
        }
        for key in ["items", "not", "if", "then", "else", "contains"] {
            if let Some(child) = schema.get(key) {
                check_schema(child);
            }
        }
        for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(values) = schema.get(key).and_then(Value::as_array) {
                for child in values {
                    check_schema(child);
                }
            }
        }
    }
    for tool in &tools.tools {
        check_schema(&serde_json::to_value(&tool.input_schema).unwrap());
    }
    let capabilities = client
        .read_resource(ReadResourceRequestParams::new("qspquest://capabilities"))
        .await
        .unwrap();
    let caps: Value = serde_json::from_str(
        serde_json::to_value(capabilities).unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let key = json!({"operation":"simulate","interface":"mcp","estimator":"none","solver":"diffsol_bdf","target":caps["target"],"execution_mode":"local_stdio"});
    let query = json!({"key":key,"model":{"model_id":"qspquest_diffdose_2026_idr_benchmark"},"scenario_id":"higher_exposure"});
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_capability")
                .with_arguments(query.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let answer = result.structured_content.unwrap();
    assert_eq!(answer["schema"], "qspquest.capability-answer/v1");
    assert_eq!(answer["active_exposed"], true);
    assert_eq!(answer["assessment"], "requires_request_validation");
    assert!(answer["executable"].is_null());
    assert_eq!(answer["qualified"], false);
    assert!(answer["model"]["archive_sha256"].as_str().is_some());
    assert!(answer["model"]["stored_request_sha256"].as_str().is_some());
    let registry =
        qspquest_mcp::server::Server::new(Library::embedded().unwrap(), false, Tier::Free, 2)
            .capability_registry();
    let advertised: std::collections::BTreeSet<&str> = registry["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["active_exposed"] == true)
        .map(|e| e["declaration"]["api"].as_str().unwrap())
        .collect();
    assert_eq!(
        advertised,
        tools
            .tools
            .iter()
            .filter(
                |t| (!t.name.starts_with("submit_") || t.name == "submit_local_job")
                    && !["preview_job", "get_job", "wait_for_job", "get_artifact"]
                        .contains(&t.name.as_ref())
            )
            .map(|t| t.name.as_ref())
            .collect()
    );
    for (name, args) in [
        ("search_models", json!({"query":"turnover"})),
        (
            "get_model",
            json!({"model_id":"qspquest_diffdose_2026_idr_benchmark"}),
        ),
        (
            "get_scenario_request",
            json!({"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}),
        ),
        ("model_template", json!({"kind":"oral"})),
        (
            "validate_model",
            json!({"source":include_str!("../assets/templates/oral.qsp")}),
        ),
        (
            "simulate",
            json!({"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}}}),
        ),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
    }
    let resources = client.list_resources(Default::default()).await.unwrap();
    assert!(resources.resources.len() > 50);
    let prompts = client.list_prompts(Default::default()).await.unwrap();
    assert_eq!(prompts.prompts.len(), 4);
    let _ = client
        .read_resource(ReadResourceRequestParams::new("qspquest://guide/language"))
        .await
        .unwrap();
    client.cancel().await.unwrap();
    task.abort();
}
#[tokio::test]
async fn selected_output_response_keeps_a_valid_browser_request() {
    let library = Library::embedded().unwrap();
    let record = library
        .records
        .iter()
        .find(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark")
        .unwrap();
    let output = record.model["outputs"][0]["name"].clone();
    let (a, b) = tokio::io::duplex(1_000_000);
    let server = Server::new(library, false, Tier::Free, 2);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let args = json!({"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark","scenario_id":"higher_exposure"}},"outputs":[output]});
    let result = client
        .call_tool(
            CallToolRequestParams::new("simulate")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let value = result.structured_content.unwrap();
    assert!(value["effective_request"]
        .get("_selected_outputs")
        .is_none());
    serde_json::from_value::<qspquest_core::run::RunRequest>(value["effective_request"].clone())
        .unwrap();
    assert_eq!(value["effective_outputs"], args["outputs"]);
    client.cancel().await.unwrap();
    task.abort();
}
#[test]
fn template_validation_and_roundtrip() {
    for k in [
        "iv_one_compartment",
        "oral",
        "two_compartment",
        "tmdd_qss",
        "idr_inhibition",
        "idr_stimulation",
        "transit",
    ] {
        let s = qspquest_mcp::server::templates(k).unwrap();
        let v =
            compute::execute(s, "validate_model", json!({})).unwrap_or_else(|e| panic!("{k}: {e}"));
        let f = v["canonical_source"].as_str().unwrap();
        let again = compute::execute(f, "validate_model", json!({})).unwrap();
        assert_eq!(v["model_content_hash"], again["model_content_hash"]);
    }
}
#[test]
fn library_hashes_and_all_model_roundtrips() {
    let lib = Library::embedded().unwrap();
    assert_eq!(lib.records.len(), 8);
    assert_eq!(
        lib.records.iter().map(|r| r.scenarios.len()).sum::<usize>(),
        47
    );
    for r in lib.records {
        let v = compute::execute_document(
            &serde_json::from_value(r.model.clone()).unwrap(),
            "validate_model",
            json!({}),
        )
        .unwrap();
        let again = compute::execute(
            v["canonical_source"].as_str().unwrap(),
            "validate_model",
            json!({}),
        )
        .unwrap();
        assert_eq!(v["model_content_hash"], again["model_content_hash"]);
    }
}
#[test]
fn admission_units_and_solver() {
    let mut r = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_rosenbrock23"});
    assert!(policy::admit(&mut r, true).is_err());
    let mut r =
        json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"events":999999}});
    assert!(policy::admit(&mut r, true).is_err());
    assert!(
        requests::dose(&json!({"target":"central","amount":100,"at":{"value":0,"unit":"h"}}))
            .is_err()
    );
    assert_eq!(Tier::Admin.scan_runs(), 400);
}
#[test]
fn summaries_preserve_event_sides_and_sampled_integral() {
    let mut r = json!({"times":[0.,1.,1.,2.],"sides":["post","pre","post","pre"],"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L","values":[2.,1.,3.,2.]}]});
    r = summary::compact(r, &Value::Null);
    assert_eq!(r["times"], json!([0., 1., 1., 2.]));
    assert_eq!(r["summary"][0]["trapezoidal_auc"], 4.);
    assert_eq!(r["summary"][0]["sampled_max"], 3.);
}
#[tokio::test]
async fn compact_preview_links_to_the_complete_grid_and_keeps_full_grid_summaries() {
    let (a, b) = tokio::io::duplex(1_000_000);
    let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let times: Vec<_> = (0..=300)
        .map(|i| json!({"value":i as f64 / 10.,"unit":"h"}))
        .collect();
    let args = json!({"model":{"source":include_str!("../assets/templates/iv.qsp")},"end":{"value":30.,"unit":"h"},"times":times,"doses":[{"target":"central","amount":{"value":100.,"unit":"mg"},"at":{"value":0.,"unit":"h"}}]});
    let preview = client
        .call_tool(
            CallToolRequestParams::new("simulate")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(preview.is_error, Some(true), "{preview:?}");
    let value = preview.structured_content.as_ref().unwrap();
    assert_eq!(value["trajectory_kind"], "compact_preview");
    assert_eq!(value["sampling"]["original_points"], 302);
    assert_eq!(value["sampling"]["returned_points"], 200);
    assert!(value["pharmacology_summary"]["method"]
        .as_str()
        .unwrap()
        .contains("not NCA"));
    let link = preview
        .content
        .iter()
        .find_map(|c| c.as_resource_link())
        .unwrap();
    assert_eq!(link.uri, value["full_result_resource"].as_str().unwrap());
    let resource = client
        .read_resource(ReadResourceRequestParams::new(&link.uri))
        .await
        .unwrap();
    let raw = serde_json::to_value(resource).unwrap();
    let full: Value = serde_json::from_str(raw["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(full["times"].as_array().unwrap().len(), 302);
    assert_eq!(full["sides"][0], "pre");
    assert_eq!(full["sides"][1], "post");
    assert_eq!(full["identity"], value["identity"]);
    assert_eq!(full["summary"], value["summary"]);
    let mut full_args = args;
    full_args["detail"] = json!("full");
    let inline = client
        .call_tool(
            CallToolRequestParams::new("simulate")
                .with_arguments(full_args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let inline = inline.structured_content.unwrap();
    assert_eq!(inline["trajectory_kind"], "full_requested_grid");
    assert_eq!(
        inline["pharmacology_summary"],
        value["pharmacology_summary"]
    );
    client.cancel().await.unwrap();
    task.abort();
}
#[tokio::test]
async fn fitting_examples_through_mcp() {
    let (a, b) = tokio::io::duplex(1_000_000);
    let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    for raw in [
        include_str!("../assets/fits/scalar.json"),
        include_str!("../assets/fits/individual-additive.json"),
        include_str!("../assets/fits/individual-proportional.json"),
        include_str!("../assets/fits/population.json"),
        include_str!("../assets/fits/library-scalar.json"),
    ] {
        let e: Value = serde_json::from_str(raw).unwrap();
        let name = e["tool"].as_str().unwrap();
        let result = client
            .call_tool(
                CallToolRequestParams::new(name.to_owned())
                    .with_arguments(e["arguments"].as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
        let v = result.structured_content.unwrap();
        let f = v.get("fit").or_else(|| v.get("population")).unwrap_or(&v);
        assert_eq!(f["status"], "converged", "{name}: {v}");
        let params = f
            .get("parameters")
            .or_else(|| f.get("fixed_effects"))
            .unwrap();
        for (key, truth) in e["expected_parameters"].as_object().unwrap() {
            assert!(
                (params[key]["value"].as_f64().unwrap() - truth.as_f64().unwrap()).abs()
                    < e["absolute_tolerance"].as_f64().unwrap(),
                "{name} {key}: {params}"
            );
        }
    }
    client.cancel().await.unwrap();
    task.abort();
}
#[test]
#[ignore = "Slow strict determinism diagnostic; independent accuracy and reviewed output bounds are checked separately"]
fn all_scenarios_native_and_sandbox_parity() {
    let lib = Library::embedded().unwrap();
    #[cfg(feature = "sandbox")]
    let wasm = qspquest_mcp::sandbox::Sandbox::embedded().unwrap();
    for r in lib.records {
        let source = r.model.to_string();
        let document = serde_json::from_value(r.model.clone()).unwrap();
        let wrapper = qspquest_modeling::document::CompiledDocument::from_json(&source).unwrap();
        let compiled = qspquest_modeling::document::CompiledDocument::from_json(&source).unwrap();
        for (id, request) in r.scenarios {
            let native =
                compute::execute_compiled(&wrapper, &document, "simulate", request.clone())
                    .unwrap_or_else(|e| panic!("{} {id}: {e}", r.metadata["id"]));
            let direct = serde_json::to_value(
                compiled
                    .execute(&serde_json::from_value(request.clone()).unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(native, direct);
            #[cfg(feature = "sandbox")]
            {
                let isolated = wasm
                    .execute_library(r.model.clone(), "simulate", request)
                    .unwrap_or_else(|e| panic!("{} {id}: {e}", r.metadata["id"]));
                assert_eq!(native["times"], isolated["times"]);
                assert_eq!(native["sides"], isolated["sides"]);
                let a = native["outputs"].as_array().unwrap();
                let b = isolated["outputs"].as_array().unwrap();
                assert_eq!(a.len(), b.len());
                for key in ["model_content_hash", "bound_value_hash", "run_request_hash"] {
                    assert_eq!(native["identity"][key], isolated["identity"][key]);
                }
                for (a, b) in a.iter().zip(b) {
                    assert_eq!(
                        a["values"].as_array().unwrap().len(),
                        b["values"].as_array().unwrap().len()
                    );
                    assert_eq!(a["name"], b["name"]);
                    assert_eq!(a["unit"], b["unit"]);
                    for (x, y) in a["values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(b["values"].as_array().unwrap())
                    {
                        let x = x.as_f64().unwrap();
                        let y = y.as_f64().unwrap();
                        assert!(
                            (x - y).abs() <= 1e-9,
                            "{} {id}: {x} vs {y}",
                            r.metadata["id"]
                        );
                    }
                }
            }
        }
    }
}
#[cfg(feature = "sandbox")]
#[test]
fn sandbox_fuel_interrupts() {
    let wasm = qspquest_mcp::sandbox::Sandbox::embedded().unwrap();
    assert!(wasm
        .limited(
            include_str!("../assets/templates/iv.qsp"),
            "validate_model",
            json!({}),
            45000,
            1
        )
        .is_err());
}

#[test]
fn selected_readouts_preserve_dynamics_and_identity_and_skip_only_unselected_readouts() {
    let library = Library::embedded().unwrap();
    let record = library
        .records
        .iter()
        .find(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark")
        .unwrap();
    let request = record.scenarios.values().next().unwrap().clone();
    let full = compute::execute_document(
        &serde_json::from_value(record.model.clone()).unwrap(),
        "simulate",
        request.clone(),
    )
    .unwrap();
    let name = full["outputs"][0]["name"].as_str().unwrap();
    let mut selected = request.clone();
    selected["_selected_outputs"] = json!([name]);
    let result = compute::execute_document(
        &serde_json::from_value(record.model.clone()).unwrap(),
        "simulate",
        selected.clone(),
    )
    .unwrap();
    assert_eq!(result["outputs"], json!([full["outputs"][0].clone()]));
    assert_eq!(result["times"], full["times"]);
    assert_eq!(result["sides"], full["sides"]);
    assert_eq!(
        result["identity"]["model_content_hash"],
        full["identity"]["model_content_hash"]
    );
    assert_ne!(
        result["identity"]["run_request_hash"],
        full["identity"]["run_request_hash"]
    );
    selected["_selected_outputs"] = json!([name, name]);
    assert!(compute::execute_document(
        &serde_json::from_value(record.model.clone()).unwrap(),
        "simulate",
        selected
    )
    .is_err());
    let mut model = record.model.clone();
    let state = model["states"][0]["name"].clone();
    model["outputs"].as_array_mut().unwrap().push(json!({"name":"undefined_readout","unit":"1","expression":{"kind":"binary","operator":"divide","arguments":[{"kind":"symbol","name":state},{"kind":"binary","operator":"subtract","arguments":[{"kind":"symbol","name":state},{"kind":"symbol","name":state}]}]}}));
    let doc = serde_json::from_value(model).unwrap();
    assert!(compute::execute_document(&doc, "simulate", request.clone()).is_err());
    let mut projected = request;
    projected["_selected_outputs"] = json!([name]);
    compute::execute_document(&doc, "simulate", projected).unwrap();
}

#[tokio::test]
async fn native_request_overrides_keep_engine_units_and_states() {
    let model: Value = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut request: Value = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-one-compartment.json"
    ))
    .unwrap();
    request["parameters"] = json!({"cl":{"value":0.3,"unit":"L/h"}});
    request["initial_states"] = json!({"central":{"value":3.0,"unit":"mg"}});
    let record = qspquest_mcp::library::Record {
        precision_profiles: None,
        metadata: json!({"id":"native_request_fixture","version":"1","candidate":false}),
        model,
        source: None,
        archive_hash: "a".repeat(64),
        fitting: Value::Null,
        scenarios: std::collections::BTreeMap::from([("preset".into(), request.clone())]),
        index: json!({"native_execution":{
            "parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}},
            "states":{"central_readout":{"native_name":"readout_central","unit":"mg"}}
        }}),
    };
    let library = Library {
        version: "fixture".into(),
        records: vec![record],
        availability_counts: Value::Null,
    };
    let (a, b) = tokio::io::duplex(1_000_000);
    let server = Server::new(library, false, Tier::Free, 2);
    let task = tokio::spawn(async move { server.serve(a).await.unwrap().waiting().await.unwrap() });
    let client = ().serve(b).await.unwrap();
    let reference =
        json!({"scenario":{"model_id":"native_request_fixture","scenario_id":"preset"}});
    let args = json!({"model":reference,"request":request,"detail":"full"});
    let response = client
        .call_tool(
            CallToolRequestParams::new("simulate")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true), "{response:?}");
    let value = response.structured_content.unwrap();
    assert_eq!(
        value["effective_request"]["parameters"],
        request["parameters"]
    );
    assert_eq!(
        value["effective_request"]["initial_states"],
        request["initial_states"]
    );
    assert!((value["outputs"][0]["values"][0].as_f64().unwrap() - 3.0).abs() < 1e-10);
    // Explicit top-level friendly overrides replace only their resolved native key.
    let mut overridden = args.clone();
    overridden["parameters"] = json!({"clearance":{"value":0.2,"unit":"mL/h"}});
    let response = client
        .call_tool(
            CallToolRequestParams::new("simulate")
                .with_arguments(overridden.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true), "{response:?}");
    let changed = response.structured_content.unwrap();
    assert_eq!(
        changed["effective_request"]["parameters"]["cl"],
        json!({"value":0.0002,"unit":"L/h"})
    );
    assert_eq!(
        changed["effective_request"]["initial_states"],
        request["initial_states"]
    );
    for (field, name, quantity) in [
        (
            "initial_states",
            "unknown_state",
            json!({"value":1,"unit":"mg"}),
        ),
        ("initial_states", "central", json!({"value":1,"unit":"h"})),
        ("parameters", "cl", json!({"value":1,"unit":"mg"})),
    ] {
        let mut bad = args.clone();
        bad["request"][field][name] = quantity;
        let response = client
            .call_tool(
                CallToolRequestParams::new("simulate")
                    .with_arguments(bad.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(response.is_error, Some(true));
    }
    client.cancel().await.unwrap();
    task.abort();
}
