use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "qspquest-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qspquest"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn model_size_budget_is_separate_from_requests_and_text() {
    use qspquest_modeling::document::{CompiledDocument, MODEL_DOCUMENT_BYTE_LIMIT};
    let temp = Temp::new();
    let mut source =
        std::fs::read_to_string(root().join("conformance/models/synthetic-one-compartment.json"))
            .unwrap();
    source.extend(std::iter::repeat_n(
        ' ',
        MODEL_DOCUMENT_BYTE_LIMIT - source.len(),
    ));
    assert!(CompiledDocument::from_json(&source).is_ok());
    let path = temp.write("large.json", &source);
    let output = run(&["validate".as_ref(), path.as_os_str()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    source.push(' ');
    assert!(CompiledDocument::from_json(&source).is_err());
    let too_large = temp.write("too-large.json", source);
    assert_eq!(
        run(&["validate".as_ref(), too_large.as_os_str()])
            .status
            .code(),
        Some(2)
    );
    let request = temp.write("oversize-request.json", vec![b' '; 1_000_001]);
    let output = run(&["run".as_ref(), path.as_os_str(), request.as_os_str()]);
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "input_limit");
}
#[test]
fn packaged_command_matches_library_scientific_identity_and_results() {
    let source = root().join("conformance/models/synthetic-pbpk-24.qsp");
    let request = root().join("conformance/requests/synthetic-pbpk-24.json");
    let output = run(&["run".as_ref(), source.as_os_str(), request.as_os_str()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let model = qspquest_modeling::language::parse(&std::fs::read_to_string(&source).unwrap())
        .unwrap()
        .compile()
        .unwrap();
    let expected = model
        .execute(&serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap())
        .unwrap();
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
}

#[test]
fn plan_matches_actual_run_and_scan_identity_without_results_or_file_changes() {
    let model = root().join("conformance/models/synthetic-one-compartment.json");
    for (request_name, execution, schema) in [
        (
            "synthetic-one-compartment.json",
            "run",
            "qspquest.run-plan/v0.1",
        ),
        ("synthetic-scan.json", "scan", "qspquest.scan-plan/v0.1"),
        ("synthetic-fit.json", "fit", "qspquest.fit-plan/v0.1"),
        (
            "synthetic-sensitivity.json",
            "sensitivities",
            "qspquest.sensitivity-plan/v0.1",
        ),
    ] {
        let request = root().join("conformance/requests").join(request_name);
        let before = std::fs::read(&request).unwrap();
        let output = run(&["plan".as_ref(), model.as_os_str(), request.as_os_str()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(plan["schema"], schema);
        let output = run(&[execution.as_ref(), model.as_os_str(), request.as_os_str()]);
        assert!(output.status.success());
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if execution == "run" {
            assert_eq!(plan["prospective_identity"], result["identity"]);
            assert!(plan["outputs"][0].get("values").is_none());
        } else if execution == "sensitivities" {
            assert_eq!(plan["prospective_identity"], result["result"]["identity"]);
            assert!(plan["derivatives"][0].get("values").is_none());
        } else if execution == "fit" {
            assert_eq!(plan["subjects"][0]["preparation"], "gaussian_objective");
            assert_eq!(plan["coordinates"][0]["parameter"], "cl");
            assert_eq!(result["fit"]["status"], "converged");
            assert!(plan.get("status").is_none() && plan.get("objective").is_none());
            for key in [
                "model_content_hash",
                "compiler_source_sha256",
                "backend_version",
            ] {
                assert_eq!(
                    plan["subjects"][0]["plan"]["prospective_identity"][key],
                    result["fit"]["objective"]["identity"][key]
                );
            }
        } else {
            assert_eq!(plan["scan_request_hash"], result["scan_request_hash"]);
            for (p, r) in plan["points"]
                .as_array()
                .unwrap()
                .iter()
                .zip(result["results"].as_array().unwrap())
            {
                assert_eq!(p["id"], r["id"]);
                assert_eq!(p["run"]["prospective_identity"], r["result"]["identity"]);
            }
        }
        assert_eq!(std::fs::read(&request).unwrap(), before);
    }
}

#[test]
fn plan_refuses_unsupported_contracts_and_retains_static_diagnostics() {
    let model = root().join("conformance/models/synthetic-one-compartment.json");
    let periodic = root().join("conformance/requests/synthetic-linear-pk-steady.json");
    let output = run(&["plan".as_ref(), model.as_os_str(), periodic.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "unsupported_plan");
    let temp = Temp::new();
    let mut request: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("conformance/requests/synthetic-one-compartment.json")).unwrap(),
    )
    .unwrap();
    request["parameters"] = serde_json::json!({"cl":{"value":1,"unit":"mg"}});
    let path = temp.write("invalid.json", serde_json::to_vec(&request).unwrap());
    let output = run(&["plan".as_ref(), model.as_os_str(), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "unit");
}
#[test]
fn format_accepts_json_roundtrips_and_never_modifies_source() {
    let path = root().join("conformance/models/synthetic-one-compartment.json");
    let before = std::fs::read(&path).unwrap();
    let output = run(&["format".as_ref(), path.as_os_str()]);
    assert!(output.status.success());
    let temp = Temp::new();
    let formatted = temp.write("formatted model.qsp", &output.stdout);
    let second = run(&["format".as_ref(), formatted.as_os_str()]);
    assert!(second.status.success());
    assert_eq!(output.stdout, second.stdout);
    let valid = run(&["validate".as_ref(), formatted.as_os_str()]);
    assert!(valid.status.success());
    let report: serde_json::Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(report["scope"], "model_compilation");
    assert_eq!(report["valid"], true);
    assert_eq!(before, std::fs::read(path).unwrap());
}
#[test]
fn scientific_and_input_failures_emit_only_structured_stderr() {
    let temp = Temp::new();
    let model = root().join("conformance/models/synthetic-invariant-loss.qsp");
    let mut request: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("conformance/requests/synthetic-invariant-loss.json")).unwrap(),
    )
    .unwrap();
    request["regimen"]["end"]["value"] = 2.into();
    let bad = temp.write("crossing.json", serde_json::to_vec(&request).unwrap());
    let output = run(&["run".as_ref(), model.as_os_str(), bad.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "invariant");
    assert!(error["error"]["time"].as_f64().unwrap() > 1.);
    let malformed = temp.write("invalid.json", b"{not json}");
    let huge = temp.write("huge.qsp", vec![b'x'; 1_000_001]);
    for args in [
        vec!["wat".as_ref()],
        vec!["run".as_ref(), model.as_os_str(), malformed.as_os_str()],
        vec!["validate".as_ref(), huge.as_os_str()],
        vec!["validate".as_ref(), temp.0.as_os_str()],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(serde_json::from_slice::<serde_json::Value>(&output.stderr).is_ok());
    }
    let help = run(&["--help".as_ref()]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
}

#[test]
fn steady_state_command_matches_typed_library_and_fails_without_output() {
    let source = root().join("conformance/models/synthetic-linear-pk.json");
    let request = root().join("conformance/requests/synthetic-linear-pk-steady.json");
    let output = run(&[
        "steady-state".as_ref(),
        source.as_os_str(),
        request.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let model = qspquest_modeling::document::CompiledDocument::from_json(
        &std::fs::read_to_string(&source).unwrap(),
    )
    .unwrap();
    let q = serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    let expected = model.periodic_steady_state(&q).unwrap();
    let planned = run(&[
        "plan-steady-state".as_ref(),
        source.as_os_str(),
        request.as_os_str(),
    ]);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(plan["prospective_identity"], actual["identity"]);
    assert_eq!(plan["request"]["operation"], "steady_state");
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
    let mut invalid: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    invalid["run"]["budgets"]["solver_callbacks"] = serde_json::json!(0);
    let tmp = Temp::new();
    let path = tmp.write("invalid.json", invalid.to_string());
    let failure = run(&[
        "steady-state".as_ref(),
        source.as_os_str(),
        path.as_os_str(),
    ]);
    assert_eq!(failure.status.code(), Some(3));
    assert!(failure.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert_eq!(error["error"]["code"], "work_budget");
}

#[test]
fn nonlinear_cycle_command_matches_typed_result() {
    let source = root().join("conformance/models/synthetic-saturable-periodic.json");
    let request = root().join("conformance/requests/synthetic-saturable-periodic.json");
    let output = run(&[
        "iterate-periodic".as_ref(),
        source.as_os_str(),
        request.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compiled = qspquest_modeling::document::CompiledDocument::from_json(
        &std::fs::read_to_string(&source).unwrap(),
    )
    .unwrap();
    let q = serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    let planned = run(&[
        "plan-iterate-periodic".as_ref(),
        source.as_os_str(),
        request.as_os_str(),
    ]);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        plan["prospective_identity"],
        result["steady_state"]["identity"]
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(compiled.iterate_periodic_state(&q).unwrap()).unwrap()
    );
}

#[test]
fn fit_command_recovers_individual_and_pooled_parameters_and_reports_limits() {
    use qspquest_core::fit::{FitProblem, FitRequest, FitStatus, PooledGaussianFitRequest};
    let source = root().join("conformance/models/synthetic-one-compartment.json");
    let mut request: FitRequest = serde_json::from_slice(
        &std::fs::read(root().join("conformance/requests/synthetic-fit.json")).unwrap(),
    )
    .unwrap();
    let document = serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
    let model = qspquest_modeling::document::sensitivity::CompiledSensitivityDocument::compile(
        &document,
        request.parameters(),
    )
    .unwrap();
    let temp = Temp::new();
    for pooled in [false, true] {
        if pooled {
            let FitProblem::Individual(fit) = request.problem.clone() else {
                panic!()
            };
            let mut second = fit.objective.clone();
            second.simulation.run.regimen.administrations[0]
                .amount
                .value *= 2.;
            for observation in &mut second.observations {
                observation.value.value *= 2.;
            }
            request.problem = FitProblem::Pooled(PooledGaussianFitRequest {
                fit,
                additional_subjects: vec![second],
            });
        }
        let path = temp.write("fit.json", serde_json::to_vec(&request).unwrap());
        let output = run(&["fit".as_ref(), source.as_os_str(), path.as_os_str()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let expected = model.execute_fit(&request).unwrap();
        let fit = expected.fit.as_ref().expect("individual or pooled fit");
        assert_eq!(fit.status, FitStatus::Converged, "{expected:?}");
        assert!((fit.parameters["cl"].value - 0.6).abs() < 1e-5);
        assert!((fit.parameters["v"].value - 4.5).abs() < 1e-5);
        assert_eq!(actual, serde_json::to_value(expected).unwrap());
    }
    let FitProblem::Pooled(ref mut pooled) = request.problem else {
        panic!()
    };
    pooled.fit.max_evaluations = 1;
    let path = temp.write("limit.json", serde_json::to_vec(&request).unwrap());
    let output = run(&["fit".as_ref(), source.as_os_str(), path.as_os_str()]);
    assert!(output.status.success());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual["fit"]["status"], "evaluation_limit");
    let mut bad = serde_json::to_value(&request).unwrap();
    bad["schema"] = "qspquest.fit/v99".into();
    let path = temp.write("bad.json", bad.to_string());
    let output = run(&["fit".as_ref(), source.as_os_str(), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn scan_command_matches_typed_results_and_returns_no_partial_results() {
    let source = root().join("conformance/models/synthetic-one-compartment.json");
    let request = root().join("conformance/requests/synthetic-scan.json");
    let output = run(&["scan".as_ref(), source.as_os_str(), request.as_os_str()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compiled = qspquest_modeling::document::CompiledDocument::from_json(
        &std::fs::read_to_string(&source).unwrap(),
    )
    .unwrap();
    let q = serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(compiled.scan(&q).unwrap()).unwrap()
    );
    let mut bad: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    bad["points"][1]["parameters"]["cl"]["unit"] = "h".into();
    let temp = Temp::new();
    let path = temp.write("invalid.json", bad.to_string());
    let output = run(&["scan".as_ref(), source.as_os_str(), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
}

#[test]
fn morris_command_preserves_generated_and_explicit_design_evidence() {
    use qspquest_core::morris::*;
    let source = root().join("conformance/models/synthetic-one-compartment.json");
    let path = root().join("conformance/requests/synthetic-morris.json");
    let mut request: MorrisExecutionRequest =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let compiled = qspquest_modeling::document::CompiledDocument::from_json(
        &std::fs::read_to_string(&source).unwrap(),
    )
    .unwrap();
    let tmp = Temp::new();
    let mut previous_effects = None;
    let mut previous_hash = None;
    for explicit in [false, true] {
        if explicit {
            let MorrisProblem::Generated(ref r) = request.problem else {
                panic!()
            };
            request.problem = MorrisProblem::Explicit(generate_morris_design(r).unwrap().request);
        }
        let path = tmp.write("morris.json", serde_json::to_vec(&request).unwrap());
        let output = run(&["morris".as_ref(), source.as_os_str(), path.as_os_str()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = compiled.execute_morris(&request).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let planned = run(&["plan".as_ref(), source.as_os_str(), path.as_os_str()]);
        assert!(
            planned.status.success(),
            "{}",
            String::from_utf8_lossy(&planned.stderr)
        );
        let plan: qspquest_core::planning::MorrisPlan =
            serde_json::from_slice(&planned.stdout).unwrap();
        assert_eq!(
            plan.analysis_request_hash,
            expected.result.analysis_request_hash
        );
        assert_eq!(
            plan.scan.scan_request_hash,
            expected.result.scan.scan_request_hash
        );
        for (point, result) in plan.scan.points.iter().zip(&expected.result.scan.results) {
            assert_eq!(point.run.prospective_identity, result.result.identity);
        }
        assert_eq!(expected.design.is_some(), !explicit);
        let effects = serde_json::to_value(&expected.result.effects).unwrap();
        if let Some(previous) = previous_effects {
            assert_eq!(previous, effects);
        }
        if let Some(previous) = previous_hash {
            assert_ne!(previous, expected.result.analysis_request_hash);
        }
        previous_effects = Some(effects);
        previous_hash = Some(expected.result.analysis_request_hash);
    }
    let mut invalid = serde_json::to_value(&request).unwrap();
    invalid["schema"] = "qspquest.morris/v99".into();
    let path = tmp.write("invalid.json", invalid.to_string());
    let output = run(&["morris".as_ref(), source.as_os_str(), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn commands_and_plans_reject_unsupported_schemas_without_science_output() {
    let temp = Temp::new();
    let model = root().join("conformance/models/synthetic-one-compartment.json");
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("bindings/mcp/assets/fits/scalar.json")).unwrap(),
    )
    .unwrap();
    let args = &fixture["arguments"];
    let scalar = serde_json::json!({
        "schema": "qspquest.scalar-fit/v0.1", "run": args["request"],
        "parameter": args["parameter"], "lower": args["bounds"]["lower"],
        "upper": args["bounds"]["upper"], "observations": args["observations"],
        "max_iterations": 100, "absolute_tolerance": 1e-8
    });
    let mut cases = vec![(scalar, "fit")];
    for (file, command) in [
        ("synthetic-one-compartment.json", "run"),
        ("synthetic-fit.json", "fit"),
        ("synthetic-sensitivity.json", "sensitivities"),
        ("synthetic-scan.json", "scan"),
        ("synthetic-morris.json", "morris"),
    ] {
        cases.push((
            serde_json::from_slice(
                &std::fs::read(root().join("conformance/requests").join(file)).unwrap(),
            )
            .unwrap(),
            command,
        ));
    }
    for (request, command) in cases {
        let tag = request["schema"].as_str().unwrap();
        let current = temp.write("current.json", request.to_string());
        let planned = run(&["plan".as_ref(), model.as_os_str(), current.as_os_str()]);
        assert!(
            planned.status.success(),
            "{}",
            String::from_utf8_lossy(&planned.stderr)
        );
        for invalid in [
            serde_json::json!(tag.replace("qspquest.", "other.")),
            serde_json::json!(tag.replace("v0.1", "v99")),
            serde_json::Value::Null,
        ] {
            let mut changed = request.clone();
            changed["schema"] = invalid;
            let path = temp.write("invalid.json", changed.to_string());
            for action in [command, "plan"] {
                let output = run(&[action.as_ref(), model.as_os_str(), path.as_os_str()]);
                assert_eq!(output.status.code(), Some(2), "{action}");
                assert!(output.stdout.is_empty(), "{action}");
                let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
                assert!(error["error"]["code"].is_string(), "{error}");
            }
        }
    }
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&model).unwrap()).unwrap();
    for tag in ["other.model/v0.1", "qspquest.model/v99"] {
        let mut changed = original.clone();
        changed["schema"] = tag.into();
        let path = temp.write("invalid-model.json", changed.to_string());
        let output = run(&["validate".as_ref(), path.as_os_str()]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(error["error"]["code"].is_string());
    }
}

#[test]
fn fit_data_runs_both_discrete_likelihoods_and_rejects_invalid_requests() {
    for (fixture, expected) in [
        ("gamma-poisson-fit.json", 14.29792892),
        ("probit-normal-binomial-fit.json", 6.59167373),
    ] {
        let request = root().join("docs/examples").join(fixture);
        let output = run(&["fit-data".as_ref(), request.as_os_str()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["status"], "converged");
        assert!((value["negative_log_likelihood"].as_f64().unwrap() - expected).abs() < 1e-7);
        assert!(value["compiler_source_sha256"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }
    let temp = Temp::new();
    for bytes in [
        br#"{"schema":"qspquest.run/v0.1"}"#.to_vec(),
        b"not json".to_vec(),
        vec![b' '; 1_000_001],
    ] {
        let path = temp.write("invalid.json", bytes);
        let output = run(&["fit-data".as_ref(), path.as_os_str()]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(error["error"]["code"].is_string());
    }
}
