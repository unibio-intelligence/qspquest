//! The model-building workflow an agent follows: copy a library model, extend it by
//! mechanism, inspect it, and compare regimens with covariates, scans and readouts.
mod common;
use qspquest_mcp::{policy::Tier, server::Server};
use rmcp::{model::CallToolRequestParams, service::RunningService, RoleClient, ServiceExt};
use serde_json::{json, Value};

const MODEL: &str = "qspquest_diffdose_2026_idr_benchmark";
const SCENARIO: &str = "higher_exposure";

const SOURCE: &str = "model covariate_turnover
time_unit = d
parameters
  cl = 0.2 [L/d] log description \"Linear clearance, synthetic\"
  v = 3 [L] log description \"Volume, synthetic\"
  kill = 0.01 [L/(mg.d)] log description \"Depletion, synthetic\"
  base = 200 [[cell]/mm3] log description \"Baseline cells, synthetic\"
  kout = 0.05 [1/d] log description \"Turnover, synthetic\"
covariates
  wt [kg] constant default 70 [kg]
individual
  v_i = v * (wt / quantity(70, \"kg\")) [L]
states
  central = 0 [mg]
  cells = (base) [[cell]/mm3]
dynamics
  d/dt(central) = input_central - cl / v_i * central
  d/dt(cells) = base * kout - kill * central / v_i * cells - kout * cells
outputs
  cp = central / v_i [mg/L]
  cell_count = cells [[cell]/mm3]
dosing
  central [mg] = 1
end
";

fn q(value: f64, unit: &str) -> Value {
    json!({"value":value,"unit":unit})
}

async fn start() -> (RunningService<RoleClient, ()>, tokio::task::JoinHandle<()>) {
    start_with(Server::new(common::library(), false, Tier::Free, 2)).await
}

async fn start_with(
    server: Server,
) -> (RunningService<RoleClient, ()>, tokio::task::JoinHandle<()>) {
    let (a, b) = tokio::io::duplex(8_000_000);
    let task = tokio::spawn(async move {
        server.serve(a).await.unwrap().waiting().await.unwrap();
    });
    (().serve(b).await.unwrap(), task)
}

#[tokio::test]
async fn hosted_edit_and_large_profile_respect_execution_admission() {
    let (client, task) = start_with(Server::new(common::library(), true, Tier::Free, 0)).await;
    let result = call(
        &client,
        "edit_model",
        json!({"source":SOURCE,"operations":[{"op":"rename","from":"cl","to":"clearance"}]}),
    )
    .await;
    assert_eq!(result["error"]["code"], "busy", "{result}");
    let rows = qspquest_mcp::policy::HOSTED_PROFILE_ROWS + 1;
    let result = call(
        &client,
        "summarize_profile",
        json!({
            "times": (1..=rows).collect::<Vec<_>>(),
            "values": (1..=rows).rev().collect::<Vec<_>>(),
            "route": "iv_bolus", "time_unit": "h", "concentration_unit": "mg/L"
        }),
    )
    .await;
    assert_eq!(result["error"]["code"], "budget", "{result}");
    client.cancel().await.unwrap();
    task.abort();
}

#[cfg(feature = "sandbox")]
#[tokio::test]
async fn hosted_edit_parses_and_validates_operations_in_the_sandbox() {
    let mut server = Server::new(common::library(), true, Tier::Free, 1);
    server.sandbox = Some(std::sync::Arc::new(
        qspquest_mcp::sandbox::Sandbox::embedded().unwrap(),
    ));
    let (client, task) = start_with(server).await;
    let result = call(&client, "edit_model", json!({
        "source": SOURCE,
        "operations": [{"op":"add_output","name":"new_cp","unit":"mg/L","expression":"central / v_i"}]
    })).await;
    assert_eq!(result["valid"], true, "{result}");
    assert!(result["canonical_source"]
        .as_str()
        .unwrap()
        .contains("new_cp"));
    client.cancel().await.unwrap();
    task.abort();
}

async fn call(client: &RunningService<RoleClient, ()>, name: &str, args: Value) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    result.structured_content.unwrap_or(Value::Null)
}

#[tokio::test]
async fn fork_edit_inspect_and_compare() {
    let (client, task) = start().await;

    // A fork reproduces its stored scenario exactly and names every symbol legally.
    let fork = call(
        &client,
        "fork_model",
        json!({"model_id":MODEL,"scenario_id":SCENARIO}),
    )
    .await;
    assert!(fork["error"].is_null(), "{fork}");
    assert_eq!(
        fork["scenario"]["equivalence"]["status"], "identical",
        "{}",
        fork["scenario"]
    );
    assert!(
        fork["scenario"]["equivalence"]["compared_values"]
            .as_u64()
            .unwrap()
            > 0
    );
    let validated = call(
        &client,
        "validate_model",
        json!({"source":fork["canonical_source"]}),
    )
    .await;
    assert_eq!(validated["valid"], true);
    for row in fork["name_map"].as_array().unwrap() {
        let name = row["readable"].as_str().unwrap();
        assert!(
            name.bytes()
                .all(|b| b == b'_' || b.is_ascii_lowercase() || b.is_ascii_digit()),
            "{name}"
        );
    }

    // An edit adds a mechanism and recompiles; a bad edit is located and refused.
    let edit = call(&client, "edit_model", json!({"source":SOURCE,"operations":[
        {"op":"add_peripheral_compartment","central":"central","name":"tissue","coordinates":"amount",
         "central_volume":"v_i","volume":q(4.0,"L"),"clearance":q(0.5,"L/d"),"description":"Synthetic assumption"},
        {"op":"add_covariate_effect","parameter":"cl","covariate":"wt","reference":q(70.0,"kg"),"form":"power","coefficient":0.75}
    ]})).await;
    assert!(edit["error"].is_null(), "{edit}");
    let extended = edit["canonical_source"].as_str().unwrap().to_owned();
    assert!(extended.contains("tissue_clearance") && extended.contains("cl_i"));
    let refused = call(
        &client,
        "edit_model",
        json!({"source":SOURCE,"operations":[{"op":"add_term","state":"absent","expression":"1"}]}),
    )
    .await;
    assert_eq!(refused["error"]["expression"], "/operations/0");

    // The inline card lists every parameter, including those reached only through v_i.
    let card = call(&client, "get_model", json!({"source":extended})).await;
    assert!(card["error"].is_null(), "{card}");
    let names: Vec<&str> = card["interface"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    for expected in [
        "v",
        "cl",
        "tissue_volume",
        "tissue_clearance",
        "cl_wt_exponent",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    assert_eq!(card["interface"]["dose_targets"][0]["target"], "central");

    // Covariates are accepted under covariates and under parameters, and change the result.
    let doses = json!([{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}]);
    let readouts = json!([{"kind":"max","output":"cp","from":q(1.0,"d"),"to":q(10.0,"d")}]);
    let light = call(&client, "simulate", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"readouts":readouts,"covariates":{"wt":q(50.0,"kg")}})).await;
    let routed = call(&client, "simulate", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"readouts":readouts,"parameters":{"wt":q(50.0,"kg")}})).await;
    let typical = call(
        &client,
        "simulate",
        json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"readouts":readouts}),
    )
    .await;
    let peak = |v: &Value| {
        v["readouts"]["results"][0]["value"]
            .as_f64()
            .unwrap_or_else(|| panic!("{v}"))
    };
    assert_eq!(peak(&light), peak(&routed));
    assert!(peak(&light) > peak(&typical));
    let unknown = call(
        &client,
        "simulate",
        json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"covariates":{"height":q(1.0,"m")}}),
    )
    .await;
    assert_eq!(unknown["error"]["expression"], "/covariates/height");

    // A readout window beyond the run end is located at the readout.
    let late = call(&client, "simulate", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"readouts":[{"kind":"max","output":"cp","from":q(1.0,"d"),"to":q(20.0,"d")}]})).await;
    assert_eq!(late["error"]["expression"], "/readouts/0/to", "{late}");

    // Scans take friendly fields and return readouts per point.
    let scan = call(&client, "scan_parameters", json!({"model":{"source":SOURCE},"parameter":"cl",
        "values":[q(0.1,"L/d"),q(0.4,"L/d")],"end":q(10.0,"d"),"doses":doses,"readouts":readouts,"covariates":{"wt":q(70.0,"kg")}})).await;
    assert!(scan["error"].is_null(), "{scan}");
    let points = scan["results"].as_array().unwrap();
    assert!(peak(&points[0]) > peak(&points[1]));

    // Comparisons keep every member's readouts inline, beyond the eight-result cache.
    let regimens: Vec<Value> = (1..=12)
        .map(|i| json!({"name":format!("dose_{i}"),"doses":[{"target":"central","amount":q(10.0 * i as f64,"mg"),"at":q(0.0,"d"),"every":q(7.0,"d"),"count":10}]}))
        .collect();
    let comparison = call(&client, "compare_regimens", json!({"model":{"source":SOURCE},"end":q(70.0,"d"),
        "regimens":regimens,"readouts":[{"kind":"min","output":"cell_count","from":q(1.0,"d"),"to":q(70.0,"d")}]})).await;
    let members = comparison["regimens"]
        .as_array()
        .unwrap_or_else(|| panic!("{comparison}"));
    assert_eq!(members.len(), 12);
    for member in members {
        assert!(
            member["result"]["readouts"]["results"][0]["value"].is_number(),
            "{}",
            member["name"]
        );
    }

    // find_dose brackets the smallest repeated dose that keeps cells at or below a ceiling.
    let found = call(&client, "find_dose", json!({"model":{"source":SOURCE},"end":q(70.0,"d"),
        "search":{"target":"central","at":q(0.0,"d"),"every":q(7.0,"d"),"count":10,"lower":q(1.0,"mg"),"upper":q(500.0,"mg")},
        "constraint":{"readout":{"kind":"max","output":"cell_count","from":q(35.0,"d"),"to":q(70.0,"d")},"operator":"at_most","threshold":100.0}})).await;
    assert_eq!(found["status"], "found", "{found}");
    let amount = found["answer"]["amount"]["value"].as_f64().unwrap();
    assert!(found["answer"]["readout"].as_f64().unwrap() <= 100.0);
    assert!(found["answer"]["bracket_readouts"][0].as_f64().unwrap() > 100.0);
    assert!(amount > 1.0 && amount < 500.0);
    let unmet = call(&client, "find_dose", json!({"model":{"source":SOURCE},"end":q(70.0,"d"),
        "search":{"target":"central","at":q(0.0,"d"),"lower":q(1.0,"mg"),"upper":q(2.0,"mg")},
        "constraint":{"readout":{"kind":"max","output":"cell_count","from":q(35.0,"d"),"to":q(70.0,"d")},"operator":"at_most","threshold":1.0}})).await;
    assert_eq!(unmet["status"], "not_met_at_upper", "{unmet}");

    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn friendly_individual_fit_recovers_clearance_from_timed_observations() {
    let (client, task) = start().await;
    let doses = json!([{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}]);
    let times = [0.5, 1.0, 2.0, 4.0, 8.0];
    let truth = call(&client, "simulate", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,
        "times":times.iter().map(|t| q(*t,"d")).collect::<Vec<_>>(),"outputs":["cp"],"detail":"full",
        "parameters":{"cl":q(0.35,"L/d")}})).await;
    let column = truth["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "cp")
        .unwrap();
    let observations: Vec<Value> = truth["times"].as_array().unwrap().iter().zip(column["values"].as_array().unwrap())
        .map(|(t, v)| json!({"time":q(t.as_f64().unwrap(),"d"),"output":"cp","value":q(v.as_f64().unwrap(),"mg/L"),
            "error":{"additive_sd":q(0.01,"mg/L"),"proportional_sd":0.0}}))
        .collect();
    let fit = call(
        &client,
        "fit_individual",
        json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,
        "parameters_to_fit":[{"parameter":"cl","lower":q(0.05,"L/d"),"upper":q(2.0,"L/d")}],
        "observations":observations}),
    )
    .await;
    assert!(fit["error"].is_null(), "{fit}");
    let estimate = fit["fit"]["parameters"]["cl"]["value"]
        .as_f64()
        .or_else(|| fit["parameters"]["cl"]["value"].as_f64())
        .unwrap_or_else(|| {
            panic!(
                "{}",
                serde_json::to_string(&fit)
                    .unwrap()
                    .chars()
                    .take(3000)
                    .collect::<String>()
            )
        });
    assert!((estimate - 0.35).abs() < 1e-3, "{estimate}");
    let mixed = call(&client, "fit_individual", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,
        "parameters_to_fit":[{"parameter":"cl","lower":q(0.05,"L/d"),"upper":q(2.0,"L/d")}],
        "observations":[observations[0].clone(), json!({"row":1,"output":"cp","value":q(1.0,"mg/L"),"error":{"additive_sd":q(0.01,"mg/L"),"proportional_sd":0.0}})]})).await;
    assert_eq!(mixed["error"]["expression"], "/observations", "{mixed}");
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn readouts_compare_with_reported_values_and_scans_explain_covariates() {
    let (client, task) = start().await;
    let doses = json!([{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}]);
    let run = call(&client, "simulate", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d"),"reported":{"value":33000.0,"unit":"ug/L","relative_tolerance":0.02}}]})).await;
    let comparison = &run["readouts"]["results"][0]["comparison"];
    assert!(comparison["within_tolerance"].is_boolean(), "{run}");
    assert!((comparison["reported_in_output_unit"].as_f64().unwrap() - 33.0).abs() < 1e-9);
    // A covariate scan runs one simulate per value; the peak scales with 1/V, and V with weight.
    let scan = call(&client, "scan_parameters", json!({"model":{"source":SOURCE},"parameter":"wt","values":[q(50.0,"kg"),q(70.0,"kg")],"end":q(10.0,"d"),"doses":doses,
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")}]})).await;
    assert!(scan["error"].is_null(), "{scan}");
    assert_eq!(scan["scanned"]["kind"], "covariate");
    let peaks: Vec<f64> = scan["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["readouts"][0]["value"].as_f64().unwrap())
        .collect();
    assert!(
        (peaks[0] - 100.0 / (3.0 * 50.0 / 70.0)).abs() < 1e-6 * peaks[0],
        "{peaks:?}"
    );
    assert!(
        (peaks[1] - 100.0 / 3.0).abs() < 1e-6 * peaks[1],
        "{peaks:?}"
    );
    let no_readouts = call(&client, "scan_parameters", json!({"model":{"source":SOURCE},"parameter":"wt","values":[q(50.0,"kg")],"end":q(10.0,"d")})).await;
    assert_eq!(no_readouts["error"]["code"], "readouts", "{no_readouts}");
    let spelled = call(&client, "scan_parameters", json!({"model":{"source":SOURCE},"parameter":"cl","values":[q(0.2,"L/day")],"end":q(10.0,"d"),"doses":doses,
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")}]})).await;
    assert!(spelled["error"].is_null(), "{spelled}");
    let card = call(&client, "get_model", json!({"source":SOURCE})).await;
    assert_eq!(card["interface"]["steady_state_supported"], false);
    let unknown = call(
        &client,
        "validate_model",
        json!({"source":SOURCE.replace("[L/d]", "[L/day]").replace("0.2 [L/day]", "0.2 [L/dy]")}),
    )
    .await;
    let text = unknown.to_string();
    assert!(text.contains("dy"), "{text}");
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn sensitivity_ranks_parameters_by_readout() {
    let (client, task) = start().await;
    let doses = json!([{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}]);
    let readouts = json!([{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")},
                          {"kind":"auc","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")}]);
    let local = call(
        &client,
        "sensitivity",
        json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"readouts":readouts,
        "vary":[{"name":"cl"},{"name":"v"},{"name":"kill"}]}),
    )
    .await;
    assert!(local["error"].is_null(), "{local}");
    let peak = &local["rankings"][0]["ranking"];
    // Bolus peak is dose / V: elasticity -1 for V, about 0 for clearance.
    let elasticity = |rows: &Value, name: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["parameter"] == name)
            .unwrap()["elasticity"]
            .as_f64()
            .unwrap()
    };
    assert!((elasticity(peak, "v") + 1.0).abs() < 0.02, "{peak}");
    assert!(elasticity(peak, "cl").abs() < 0.02);
    assert_eq!(peak[0]["parameter"], "v");
    let morris = call(&client, "sensitivity", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"readouts":readouts,
        "method":"morris","trajectories":4,"seed":7,
        "vary":[{"name":"cl","lower":q(0.1,"L/d"),"upper":q(0.4,"L/d")},{"name":"v","lower":q(2.0,"L"),"upper":q(4.0,"L")}]})).await;
    assert!(morris["error"].is_null(), "{morris}");
    assert_eq!(morris["simulations"], 1 + 4 * 3);
    assert!(
        morris["rankings"][1]["ranking"][0]["mu_star"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn dimension_errors_name_both_sides_and_the_source_line() {
    let (client, task) = start().await;
    // cl / v_i * central is amount/time; cl * central / v_i is the same, but cl * central is not.
    let broken = SOURCE.replace(
        "d/dt(central) = input_central - cl / v_i * central",
        "d/dt(central) = input_central - cl * central",
    );
    let result = call(&client, "validate_model", json!({"source":broken})).await;
    let text = result.to_string();
    assert!(text.contains("versus"), "{text}");
    assert!(text.contains("\"line\":17"), "{text}");
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn verification_round_fixes_hold() {
    let (client, task) = start().await;
    let doses = json!([{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}]);
    // get_model takes the same model reference as the other tools.
    let card = call(&client, "get_model", json!({"model":{"source":SOURCE}})).await;
    assert_eq!(card["model_source"], "inline_source", "{card}");
    // Comparison receipts echo friendly covariates.
    let cmp = call(
        &client,
        "compare_regimens",
        json!({"model":{"source":SOURCE},"end":q(10.0,"d"),
        "regimens":[{"name":"heavy","doses":doses,"covariates":{"wt":q(90.0,"kg")}}],
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")}]}),
    )
    .await;
    assert_eq!(
        cmp["regimens"][0]["input_receipt"]["requested_inputs"]["covariates"]["wt"]["value"], 90.0,
        "{cmp}"
    );
    // find_dose reports the readout unit and what it held constant.
    let found = call(&client, "find_dose", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"covariates":{"wt":q(90.0,"kg")},
        "search":{"target":"central","at":q(0.0,"d"),"lower":q(1.0,"mg"),"upper":q(500.0,"mg")},
        "constraint":{"readout":{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")},"operator":"at_least","threshold":5.0}})).await;
    assert_eq!(found["readout_unit"], "mg/L", "{found}");
    assert_eq!(found["held_constant"]["covariates"]["wt"]["value"], 90.0);
    // Morris converts the upper bound into the lower bound's unit.
    let morris = call(&client, "sensitivity", json!({"model":{"source":SOURCE},"end":q(10.0,"d"),"doses":doses,"method":"morris","trajectories":2,
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(10.0,"d")}],
        "vary":[{"name":"v","lower":q(2000.0,"mL"),"upper":q(4.0,"L")}]})).await;
    assert!(morris["error"].is_null(), "{morris}");
    assert_eq!(morris["rankings"][0]["ranking"][0]["unit"], "mg/L");
    // set_initial on a preloaded state names constant outputs.
    let preloaded = SOURCE
        .replace("central = 0 [mg]", "central = 100 [mg]")
        .replace(
            "cell_count = cells [[cell]/mm3]",
            "cell_count = cells [[cell]/mm3]\n  start_conc = base * kout [[cell]/(mm3.d)]",
        );
    let edit = call(&client, "edit_model", json!({"source":preloaded,"operations":[{"op":"set_initial","state":"central","initial":q(0.0,"mg")}]})).await;
    assert!(
        edit["changes"]["applied"][0]["summary"]
            .as_str()
            .unwrap()
            .contains("start_conc"),
        "{edit}"
    );
    // A lag written on the dosing line gets the lag(...) hint.
    let lagged = call(
        &client,
        "validate_model",
        json!({"source":SOURCE.replace("central [mg] = 1", "central [mg] = 1 lag tlag")}),
    )
    .await;
    assert!(lagged.to_string().contains("lag(depot) = tlag"), "{lagged}");
    // Empty searches say how to widen them.
    let empty = call(&client, "search_models", json!({"query":"zzzz"})).await;
    assert!(
        empty["hint"]
            .as_str()
            .unwrap()
            .starts_with("No model matched"),
        "{empty}"
    );
    client.cancel().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn steady_state_nca_and_dose_scans_for_any_model() {
    let (client, task) = start().await;
    // Steady state by repeated intervals for a model without an analytic path. One
    // compartment, k = cl/v = 0.2/3 per day, 100 mg every 7 d: R = 1/(1 - exp(-7k)),
    // trough = (100/3) R exp(-7k), AUC over the interval = dose/CL = 500 mg.d/L.
    let ss = call(
        &client,
        "simulate_steady_state",
        json!({"model":{"source":SOURCE},"dose":{"target":"central","amount":q(100.0,"mg")},
        "interval":q(7.0,"d"),"outputs":["cp"],"relative_tolerance":1e-4}),
    )
    .await;
    assert_eq!(ss["status"], "steady_state", "{ss}");
    let row = &ss["steady_state"][0];
    let e = (-7.0f64 * 0.2 / 3.0).exp();
    let ratio = 1.0 / (1.0 - e);
    let close = |got: &Value, want: f64, tol: f64| {
        assert!(
            (got.as_f64().unwrap() - want).abs() <= tol * want,
            "{got} vs {want}: {row}"
        )
    };
    close(&row["accumulation_ratio"], ratio, 2e-3);
    close(&row["trough"]["value"], 100.0 / 3.0 * ratio * e, 2e-3);
    close(&row["cmax"]["value"], 100.0 / 3.0 * ratio, 2e-3);
    close(&row["auc_tau"]["value"], 500.0, 2e-3);
    assert_eq!(row["trough"]["unit"], "mg/L");
    close(&row["cavg"]["value"], 500.0 / 7.0, 2e-3);

    // An hourly model with the interval in days and strong accumulation (R about 50): the
    // average uses the interval in model time, and the reported trough is within the tolerance
    // of the analytic steady state, not just close to the previous interval.
    let hourly = SOURCE
        .replace("time_unit = d", "time_unit = h")
        .replace("0.2 [L/d]", "0.0025 [L/h]");
    let ss = call(
        &client,
        "simulate_steady_state",
        json!({"model":{"source":hourly},"dose":{"target":"central","amount":q(100.0,"mg")},
        "interval":q(1.0,"d"),"outputs":["cp"],"relative_tolerance":1e-3,"max_intervals":2000}),
    )
    .await;
    assert_eq!(ss["status"], "steady_state", "{ss}");
    let row = &ss["steady_state"][0];
    let k = 0.0025 / 3.0;
    let e = (-24.0f64 * k).exp();
    let ratio = 1.0 / (1.0 - e);
    assert!(ratio > 45.0 && ratio < 55.0, "{ratio}");
    let close = |got: &Value, want: f64, tol: f64| {
        assert!(
            (got.as_f64().unwrap() - want).abs() <= tol * want,
            "{got} vs {want}: {row}"
        )
    };
    close(&row["trough"]["value"], 100.0 / 3.0 * ratio * e, 2e-3);
    close(&row["auc_tau"]["value"], 100.0 / 0.0025, 2e-3);
    close(&row["cavg"]["value"], 100.0 / 0.0025 / 24.0, 2e-3);
    assert_eq!(row["cavg"]["unit"], "mg/L");
    assert!(
        row["auc_tau"]["unit"].as_str().unwrap().contains("mg"),
        "{row}"
    );

    // Full NCA on a supplied profile: an exact exponential with C0 = 10 mg/L and k = 0.1/h
    // after 100 mg IV gives AUCIFO = 100 mg.h/L, CL = 1 L/h, Vz = 10 L.
    let times: Vec<f64> = [0.5, 1.0, 2.0, 4.0, 6.0, 8.0, 12.0, 24.0].to_vec();
    let values: Vec<f64> = times.iter().map(|t| 10.0 * (-0.1 * t).exp()).collect();
    let profile = call(
        &client,
        "summarize_profile",
        json!({"times":times,"values":values,"time_unit":"h","concentration_unit":"mg/L",
        "route":"iv_bolus","dose":q(100.0,"mg")}),
    )
    .await;
    let nca = &profile["nca"]["parameters"];
    assert!(nca.is_object(), "{profile}");
    for (code, want, unit) in [
        ("AUCIFO", 100.0, "mg/L.h"),
        ("LAMZ", 0.1, "1/h"),
        ("CLO", 1.0, "L/h"),
        ("VZO", 10.0, "L"),
    ] {
        let got = nca[code]["value"].as_f64().unwrap();
        assert!((got - want).abs() < 1e-6 * want, "{code}: {got} vs {want}");
        assert_eq!(nca[code]["unit"], unit, "{code}");
    }
    assert!(nca["AUCPEO"]["value"].as_f64().unwrap() < 50.0);
    for (code, entry) in nca.as_object().unwrap() {
        assert!(entry["unit"].is_string(), "{code} has no unit: {entry}");
    }
    assert!(
        profile["results"][0]["value"].get("auc").is_none(),
        "{profile}"
    );
    let unitless = call(
        &client,
        "summarize_profile",
        json!({"times":times,"values":values,"route":"iv_bolus"}),
    )
    .await;
    assert_eq!(unitless["error"]["code"], "units", "{unitless}");

    // The nca readout on a simulation gives the same single-dose parameters.
    let grid: Vec<Value> = (0..=56).map(|i| q(i as f64 * 0.5, "d")).collect();
    let run = call(&client, "simulate", json!({"model":{"source":SOURCE},"times":grid,
        "doses":[{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}],
        "readouts":[{"kind":"nca","output":"cp","from":q(0.0,"d"),"to":q(28.0,"d"),"route":"iv_bolus","dose":q(100.0,"mg")}]})).await;
    let single = &run["readouts"]["results"][0]["single_dose_nca"]["parameters"];
    assert!(single.is_object(), "{run}");
    assert!(
        (single["CLO"]["value"].as_f64().unwrap() - 0.2).abs() < 2e-3,
        "{single}"
    );
    assert!(
        (single["VZO"]["value"].as_f64().unwrap() - 3.0).abs() < 3e-2,
        "{single}"
    );
    assert_eq!(single["CLO"]["unit"], "L/d");
    // One AUC per row: the grid trapezoid gives way to the NCA.
    assert!(
        run["readouts"]["results"][0]["value"].get("auc").is_none(),
        "{run}"
    );
    // A window that does not start at the bolus is refused rather than back-extrapolated.
    let late = call(&client, "simulate", json!({"model":{"source":SOURCE},"times":grid,
        "doses":[{"target":"central","amount":q(100.0,"mg"),"at":q(0.0,"d")}],
        "readouts":[{"kind":"nca","output":"cp","from":q(1.0,"d"),"to":q(28.0,"d"),"route":"iv_bolus","dose":q(100.0,"mg")}]})).await;
    assert!(late["error"].to_string().contains("window start"), "{late}");

    // mg/kg doses resolve against body weight and the note records the amount.
    let per_kg = call(
        &client,
        "simulate",
        json!({"model":{"source":SOURCE},"end":q(1.0,"d"),"covariates":{"wt":q(50.0,"kg")},
        "doses":[{"target":"central","amount":q(2.0,"mg/kg"),"at":q(0.0,"d")}],
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(1.0,"d")}]}),
    )
    .await;
    assert!(per_kg["error"].is_null(), "{per_kg}");
    let notes = per_kg["notes"].to_string();
    assert!(
        notes.contains("= 100 mg") && notes.contains("covariate wt"),
        "{notes}"
    );
    let defaulted = call(
        &client,
        "simulate",
        json!({"model":{"source":SOURCE},"end":q(1.0,"d"),
        "doses":[{"target":"central","amount":q(1.0,"mg/kg"),"at":q(0.0,"d")}]}),
    )
    .await;
    assert!(
        defaulted["notes"]
            .to_string()
            .contains("model default of covariate wt"),
        "{defaulted}"
    );

    // A dose-amount scan in mg/kg: amounts follow weight; the per-weight peak does not.
    let scan = call(&client, "scan_parameters", json!({"model":{"source":SOURCE},"dose_index":0,"values":[q(1.0,"mg/kg"),q(2.0,"mg/kg")],
        "end":q(1.0,"d"),"covariates":{"wt":q(50.0,"kg")},"doses":[{"target":"central","amount":q(1.0,"mg"),"at":q(0.0,"d")}],
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(1.0,"d")}]})).await;
    assert!(scan["error"].is_null(), "{scan}");
    assert_eq!(scan["scanned"]["kind"], "dose_amount");
    let points = scan["points"].as_array().unwrap();
    assert!(
        points[1]["resolved_doses"].to_string().contains("= 100 mg"),
        "{scan}"
    );
    let peak = points[1]["readouts"][0]["value"].as_f64().unwrap();
    assert!(
        (peak - 100.0 / (3.0 * 50.0 / 70.0)).abs() < 1e-6 * peak,
        "{peak}"
    );
    // compare_regimens resolves a per-kilogram dose against each member's own weight.
    let members = json!([
        {"name":"light","covariates":{"wt":q(50.0,"kg")},"doses":[{"target":"central","amount":q(2.0,"mg/kg"),"at":q(0.0,"d")}]},
        {"name":"heavy","covariates":{"wt":q(100.0,"kg")},"doses":[{"target":"central","amount":q(2.0,"mg/kg"),"at":q(0.0,"d")}]}]);
    let cmp = call(
        &client,
        "compare_regimens",
        json!({"model":{"source":SOURCE},"end":q(1.0,"d"),"regimens":members,
        "readouts":[{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(1.0,"d")}]}),
    )
    .await;
    let text = cmp.to_string();
    assert!(
        text.contains("= 100 mg") && text.contains("= 200 mg"),
        "{cmp}"
    );
    let out_of_range = call(
        &client,
        "scan_parameters",
        json!({"model":{"source":SOURCE},"dose_index":3,"values":[q(1.0,"mg")],"end":q(1.0,"d"),
        "doses":[{"target":"central","amount":q(1.0,"mg"),"at":q(0.0,"d")}],"output":"cp"}),
    )
    .await;
    assert_eq!(
        out_of_range["error"]["code"], "dose_index",
        "{out_of_range}"
    );
    // A ceiling met at all sampled doses does not establish an outside boundary.
    let ceiling = call(&client, "find_dose", json!({"model":{"source":SOURCE},"end":q(2.0,"d"),
        "search":{"target":"central","at":q(0.0,"d"),"lower":q(1.0,"mg"),"upper":q(10.0,"mg")},
        "constraint":{"readout":{"kind":"max","output":"cp","from":q(0.0,"d"),"to":q(2.0,"d")},"operator":"at_most","threshold":1000.0}})).await;
    assert_eq!(ceiling["status"], "met_at_lower", "{ceiling}");
    assert!(
        ceiling["explanation"]
            .as_str()
            .unwrap()
            .contains("Other amounts have not been assessed"),
        "{ceiling}"
    );
    let parabolic = SOURCE.replace("input_central - cl / v_i * central", "input_central")
        .replace("  cell_count = cells", "  dose_response = (central / quantity(10, \"mg\")) * (1 - central / quantity(10, \"mg\")) [1]\n  cell_count = cells");
    let interior = call(&client, "find_dose", json!({"model":{"source":parabolic},"end":q(1.0,"d"),
        "search":{"target":"central","at":q(0.0,"d"),"lower":q(0.0,"mg"),"upper":q(10.0,"mg")},
        "constraint":{"readout":{"kind":"max","output":"dose_response","from":q(0.0,"d"),"to":q(1.0,"d")},"operator":"at_least","threshold":0.2}})).await;
    assert_eq!(interior["status"], "not_monotonic", "{interior}");
    assert!(interior["trace"][2]["readout"].as_f64().unwrap() >= 0.2);
    // plan_run cannot bind a per-point scan and says so.
    let planned = call(&client, "plan_run", json!({"operation":"scan_parameters","arguments":{"model":{"source":SOURCE},"dose_index":0,"values":[q(1.0,"mg")],
        "end":q(1.0,"d"),"doses":[{"target":"central","amount":q(1.0,"mg"),"at":q(0.0,"d")}],"output":"cp"}})).await;
    assert_eq!(planned["error"]["code"], "scan_dose_amount", "{planned}");
    let planned = call(&client, "plan_run", json!({"operation":"scan_parameters","arguments":{"model":{"source":SOURCE},"parameter":"wt","values":[q(50.0,"kg")],"end":q(1.0,"d"),"output":"cp"}})).await;
    assert_eq!(planned["error"]["code"], "scan_covariate", "{planned}");
    client.cancel().await.unwrap();
    task.abort();
}
