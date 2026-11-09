//! Inline workload admission. Native preparation remains the scientific authority.
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Profile {
    Local,
    Hosted,
}
impl Profile {
    pub(crate) fn from_hosted(hosted: bool) -> Self {
        if hosted {
            Self::Hosted
        } else {
            Self::Local
        }
    }
    pub(crate) fn population_limits(self) -> (usize, usize) {
        match self {
            Self::Hosted => (16, 256),
            Self::Local => (128, 10_000),
        }
    }
    fn optimization_limit(self, field: &str) -> u64 {
        match (self, field) {
            (Self::Hosted, "max_evaluations") => 2000,
            (Self::Hosted, _) => 300,
            (Self::Local, "max_evaluations") => 100_000,
            (Self::Local, _) => 10_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct WorkLimits {
    solver_callbacks: u64,
    events: u64,
    output_values: u64,
}
const RUN: WorkLimits = WorkLimits {
    solver_callbacks: 1_000_000,
    events: 10_000,
    output_values: 25_000_000,
};
const TOTAL: WorkLimits = WorkLimits {
    solver_callbacks: 25_000_000,
    events: 250_000,
    output_values: 625_000_000,
};
impl WorkLimits {
    fn fields(self) -> [(&'static str, u64); 3] {
        [
            ("solver_callbacks", self.solver_callbacks),
            ("events", self.events),
            ("output_values", self.output_values),
        ]
    }
    fn fill(self, fields: &mut Map<String, Value>) {
        for (name, limit) in self.fields() {
            fields.entry(name).or_insert_with(|| json!(limit));
        }
    }
}

#[derive(Debug)]
enum Reason {
    InputLimit,
    WithheldSolver,
    Object(&'static str),
    Array(&'static str),
    SubjectObject,
    PositiveInteger {
        field: &'static str,
        maximum: u64,
        aggregate: bool,
    },
    SubjectCount {
        maximum: usize,
    },
    ObservationCount {
        maximum: usize,
    },
    RandomEffectCount,
}
#[derive(Debug)]
pub(crate) struct Failure {
    pointer: String,
    reason: Reason,
}
impl Failure {
    fn at(pointer: String, reason: Reason) -> Self {
        Self { pointer, reason }
    }
    pub(crate) fn into_value(self) -> Value {
        let (code, message, suggestion) = match self.reason {
            Reason::InputLimit => ("input_limit", "Tool arguments exceed 1 MB".to_owned(),
                "Reduce the submitted request to at most 1000000 serialized UTF-8 bytes.".to_owned()),
            Reason::WithheldSolver => ("unsupported_solver",
                "Rosenbrock23 and Rodas5P are unavailable; choose a supported standard Diffsol solver".to_owned(),
                "Choose diffsol_bdf for simulation, sensitivities or fitting.".to_owned()),
            Reason::Object(field) => ("invalid_input", format!("{field} must be an object"),
                format!("Supply {field} as an object with solver_callbacks, events and output_values integer fields.")),
            Reason::Array(field) => ("invalid_input", format!("{field} must be an array"),
                format!("Supply {field} as an array; inspect the requested native operation's schema for its entries.")),
            Reason::SubjectObject => ("invalid_input", "Each subject must be an object".to_owned(),
                "Supply a complete native subject objective with simulation, observations and priors; inspect the population request schema.".to_owned()),
            Reason::PositiveInteger { field, maximum, aggregate } => ("budget",
                if aggregate { format!("total_budgets.{field} exceeds the bounded inline workload") }
                else { format!("{field} must be 1..{maximum}") },
                format!("Set this field to an integer from 1 through {maximum}. Reduce the workload or choose an explicitly supported local or governed job route.")),
            Reason::SubjectCount { maximum } => ("budget", format!("population subjects must be 1..{maximum}"),
                format!("Supply 1 through {maximum} subjects for this execution profile; retain a scientifically justified subject set.")),
            Reason::ObservationCount { maximum } => ("budget", "population observation limit exceeded".to_owned(),
                format!("The combined subject observation count must be at most {maximum}; use an explicitly supported larger-workload route when all study observations are required.")),
            Reason::RandomEffectCount => ("unsupported",
                "Native population fitting supports at most eight random effects (SAEM); FOCEI and Laplace support fewer, and the engine checks each method's constraints".to_owned(),
                "Use no more random effects than the data identify; native preparation checks estimator-specific constraints.".to_owned()),
        };
        json!({"error":{"code":code,"message":message,"expression":self.pointer,"suggestion":suggestion}})
    }
}

fn child(path: &str, name: &str) -> String {
    format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"))
}
fn positive_integer(
    value: &Value,
    path: String,
    field: &'static str,
    maximum: u64,
    aggregate: bool,
) -> Result<(), Failure> {
    if value.as_u64().is_none_or(|n| n == 0 || n > maximum) {
        return Err(Failure::at(
            path,
            Reason::PositiveInteger {
                field,
                maximum,
                aggregate,
            },
        ));
    }
    Ok(())
}
fn budgets(
    value: &Value,
    path: &str,
    field: &'static str,
    limits: WorkLimits,
    aggregate: bool,
) -> Result<(), Failure> {
    let object = value
        .as_object()
        .ok_or_else(|| Failure::at(path.to_owned(), Reason::Object(field)))?;
    for (name, maximum) in limits.fields() {
        if let Some(value) = object.get(name) {
            positive_integer(value, child(path, name), name, maximum, aggregate)?;
        }
    }
    Ok(())
}
// Binding maps contain caller-selected scientific names, not workload fields.
// A parameter named 'subjects' or 'max_iterations' is still a native quantity.
fn binding_map(name: &str, value: &Value) -> bool {
    matches!(name, "parameters" | "initial_states" | "covariates") && value.is_object()
}
fn validate(
    value: &Value,
    profile: Profile,
    path: &str,
    saem_evaluation_path: Option<&str>,
) -> Result<(), Failure> {
    if let Some(object) = value.as_object() {
        // Native SAEM with two to eight effects supports an explicit 200,000
        // evaluation ceiling. Scope this local allowance to its exact field;
        // individual, one-effect and hosted request limits remain separate.
        let two_effect_saem = value["schema"] == "qspquest.fit/v0.1"
            && value["problem"]["kind"] == "saem"
            && value["problem"]["request"]["random_effects"]
                .as_array()
                .is_some_and(|effects| (2..=8).contains(&effects.len()));
        let declared_path = two_effect_saem.then(|| {
            child(
                &child(&child(path, "problem"), "request"),
                "max_evaluations",
            )
        });
        let saem_evaluation_path = declared_path.as_deref().or(saem_evaluation_path);
        if matches!(
            object.get("solver").and_then(Value::as_str),
            Some("diffsol_rosenbrock23" | "diffsol_rodas5p")
        ) {
            return Err(Failure::at(child(path, "solver"), Reason::WithheldSolver));
        }
        if object.get("schema").and_then(Value::as_str) == Some("qspquest.run/v0.1") {
            if let Some(value) = object.get("budgets") {
                budgets(value, &child(path, "budgets"), "budgets", RUN, false)?;
            }
        }
        if let Some(value) = object.get("total_budgets") {
            budgets(
                value,
                &child(path, "total_budgets"),
                "total_budgets",
                TOTAL,
                true,
            )?;
        }
        for field in ["max_evaluations", "max_iterations"] {
            if let Some(value) = object.get(field) {
                let at = child(path, field);
                let maximum = match profile {
                    Profile::Local
                        if field == "max_evaluations"
                            && saem_evaluation_path == Some(at.as_str()) =>
                    {
                        200_000
                    }
                    _ => profile.optimization_limit(field),
                };
                positive_integer(value, at, field, maximum, false)?;
            }
        }
        if let Some(value) = object.get("subjects") {
            let path = child(path, "subjects");
            let subjects = value
                .as_array()
                .ok_or_else(|| Failure::at(path.clone(), Reason::Array("subjects")))?;
            let (maximum, observation_maximum) = profile.population_limits();
            if subjects.is_empty() || subjects.len() > maximum {
                return Err(Failure::at(path, Reason::SubjectCount { maximum }));
            }
            let mut count = 0usize;
            for (index, subject) in subjects.iter().enumerate() {
                if !subject.is_object() {
                    return Err(Failure::at(
                        child(&path, &index.to_string()),
                        Reason::SubjectObject,
                    ));
                }
                if let Some(value) = subject.get("observations") {
                    let at = child(&child(&path, &index.to_string()), "observations");
                    let observations = value
                        .as_array()
                        .ok_or_else(|| Failure::at(at, Reason::Array("observations")))?;
                    count = count.checked_add(observations.len()).ok_or_else(|| {
                        Failure::at(
                            path.clone(),
                            Reason::ObservationCount {
                                maximum: observation_maximum,
                            },
                        )
                    })?;
                    if count > observation_maximum {
                        return Err(Failure::at(
                            path,
                            Reason::ObservationCount {
                                maximum: observation_maximum,
                            },
                        ));
                    }
                }
            }
        }
        if let Some(value) = object.get("random_effects") {
            let at = child(path, "random_effects");
            let effects = value
                .as_array()
                .ok_or_else(|| Failure::at(at.clone(), Reason::Array("random_effects")))?;
            if effects.len() > 8 {
                return Err(Failure::at(at, Reason::RandomEffectCount));
            }
        }
        for (name, value) in object {
            if !binding_map(name, value) {
                validate(value, profile, &child(path, name), saem_evaluation_path)?;
            }
        }
    } else if let Some(array) = value.as_array() {
        for (index, value) in array.iter().enumerate() {
            validate(
                value,
                profile,
                &child(path, &index.to_string()),
                saem_evaluation_path,
            )?;
        }
    }
    Ok(())
}
fn fill_defaults(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.get("schema").and_then(Value::as_str) == Some("qspquest.run/v0.1") {
            let budget = object.entry("budgets").or_insert_with(|| json!({}));
            if let Some(fields) = budget.as_object_mut() {
                RUN.fill(fields);
            }
        }
        for (name, value) in object {
            if !binding_map(name, value) {
                fill_defaults(value);
            }
        }
    } else if let Some(array) = value.as_array_mut() {
        for value in array {
            fill_defaults(value);
        }
    }
}

/// Hosted ceiling on the aggregate simulations of one analysis request.
pub(crate) const HOSTED_ANALYSIS_SIMULATIONS: u64 = 500;

/// Locations of the run requests embedded in an analysis envelope.
fn analysis_runs(operation: &str, arguments: &Value) -> Vec<String> {
    let indexed = |field: &str| -> Vec<String> {
        arguments[field]
            .as_array()
            .map(|items| {
                (0..items.len())
                    .map(|i| format!("/arguments/{field}/{i}/run"))
                    .collect()
            })
            .unwrap_or_default()
    };
    match operation {
        "fit_data" | "profile_likelihood" | "event_sensitivities" => vec!["/arguments/run".into()],
        "fit_population" | "visual_predictive_check" | "bootstrap_population" => {
            indexed("subjects")
        }
        "compare_regimens" => indexed("regimens"),
        _ => Vec::new(),
    }
}

/// Admit a `qspquest.analysis/v0.2` envelope. Every embedded run obeys the
/// per-run limits of the profile. A hosted request must state
/// `max_simulations`, at most [`HOSTED_ANALYSIS_SIMULATIONS`], runs on one
/// worker, and its per-run budgets multiplied by `max_simulations` must stay
/// within the aggregate inline workload. Run budget defaults are filled in so
/// the plan shows the effective request. Native analysis owns every other
/// argument; the generic walker is not applied, because analysis fields such
/// as an integer `subjects` or up to eight `random_effects` have their own
/// meaning.
pub(crate) fn admit_analysis(value: &mut Value, profile: Profile) -> Result<(), Value> {
    let located = |pointer: &str, code: &str, message: String, suggestion: &str| json!({"error":{"code":code,"message":message,"expression":pointer,"suggestion":suggestion}});
    if value.to_string().len() > crate::policy::INPUT_LIMIT {
        return Err(Failure::at(String::new(), Reason::InputLimit).into_value());
    }
    let operation = value["operation"].as_str().unwrap_or("").to_owned();
    let arguments = &value["arguments"];
    let hosted = matches!(profile, Profile::Hosted);
    if !crate::core::analysis_request::RECORD_OPERATIONS.contains(&operation.as_str()) {
        let maximum = match profile {
            Profile::Hosted => HOSTED_ANALYSIS_SIMULATIONS,
            Profile::Local => 1_000_000,
        };
        let declared = arguments.get("max_simulations").filter(|v| !v.is_null());
        match declared {
            None if hosted => {
                return Err(located(
                    "/arguments/max_simulations",
                    "budget",
                    format!("Hosted analyses must state max_simulations, at most {maximum}"),
                    "Set max_simulations to the simulations the analysis needs (see the plan's required_simulations), or run locally.",
                ))
            }
            Some(v) if v.as_u64().is_none_or(|n| n == 0 || n > maximum) => {
                return Err(located(
                    "/arguments/max_simulations",
                    "budget",
                    format!("max_simulations must be 1..{maximum}"),
                    "Reduce the analysis workload, or run it with local stdio where the native ceiling of 1000000 applies.",
                ))
            }
            _ => {}
        }
        if hosted
            && arguments
                .get("workers")
                .filter(|v| !v.is_null())
                .is_some_and(|v| v.as_u64() != Some(1))
        {
            return Err(located(
                "/arguments/workers",
                "budget",
                "Hosted analyses run on one worker".into(),
                "Omit workers or set it to 1; results do not depend on the worker count.",
            ));
        }
        let simulations = declared.and_then(Value::as_u64).unwrap_or(maximum);
        let mut admitted = value.clone();
        for pointer in analysis_runs(&operation, arguments) {
            let Some(run) = admitted.pointer_mut(&pointer) else {
                continue;
            };
            validate(run, profile, &pointer, None).map_err(Failure::into_value)?;
            fill_defaults(run);
            if hosted {
                for (name, limit) in TOTAL.fields() {
                    let per_run = run["budgets"][name].as_u64().unwrap_or(u64::MAX);
                    if per_run.saturating_mul(simulations) > limit {
                        return Err(located(
                            &format!("{pointer}/budgets/{name}"),
                            "budget",
                            format!("budgets.{name} times max_simulations exceeds the hosted aggregate of {limit}"),
                            "Lower this run budget or max_simulations so their product stays within the hosted aggregate, or run locally.",
                        ));
                    }
                }
            }
        }
        if admitted.to_string().len() > crate::policy::INPUT_LIMIT {
            return Err(Failure::at(String::new(), Reason::InputLimit).into_value());
        }
        *value = admitted;
    }
    Ok(())
}

pub(crate) fn admit(value: &mut Value, profile: Profile) -> Result<(), Failure> {
    if value.to_string().len() > crate::policy::INPUT_LIMIT {
        return Err(Failure::at(String::new(), Reason::InputLimit));
    }
    validate(value, profile, "", None)?;
    let mut admitted = value.clone();
    fill_defaults(&mut admitted);
    if admitted.to_string().len() > crate::policy::INPUT_LIMIT {
        return Err(Failure::at(String::new(), Reason::InputLimit));
    }
    *value = admitted;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_keeps_successful_budget_defaults_and_idempotence() {
        let mut request = json!({"schema":"qspquest.scan/v0.1","run":{"schema":"qspquest.run/v0.1","budgets":{"events":7}},"total_budgets":{"events":20}});
        admit(&mut request, Profile::Hosted).unwrap();
        assert_eq!(
            request["run"]["budgets"],
            json!({"solver_callbacks":1_000_000,"events":7,"output_values":25_000_000})
        );
        assert_eq!(request["total_budgets"], json!({"events":20}));
        let once = request.clone();
        admit(&mut request, Profile::Hosted).unwrap();
        assert_eq!(request, once);
    }

    #[test]
    fn refusal_is_atomic_after_valid_nested_runs() {
        let mut request = json!({"a":{"schema":"qspquest.run/v0.1"},"z":{"max_iterations":301}});
        let before = request.clone();
        let failure = admit(&mut request, Profile::Hosted)
            .unwrap_err()
            .into_value();
        assert_eq!(request, before);
        assert_eq!(failure["error"]["expression"], "/z/max_iterations");
        assert_eq!(failure["error"]["code"], "budget");
    }

    #[test]
    fn malformed_budget_and_population_containers_are_located() {
        for (mut request, expected) in [
            (
                json!({"schema":"qspquest.run/v0.1","budgets":null}),
                "/budgets",
            ),
            (json!({"total_budgets":3}), "/total_budgets"),
            (json!({"problem":{"subjects":{}}}), "/problem/subjects"),
            (json!({"subjects":[false]}), "/subjects/0"),
            (
                json!({"subjects":[{"observations":null}]}),
                "/subjects/0/observations",
            ),
            (json!({"random_effects":"eta"}), "/random_effects"),
        ] {
            let before = request.clone();
            let failure = admit(&mut request, Profile::Hosted)
                .unwrap_err()
                .into_value();
            assert_eq!(failure["error"]["expression"], expected);
            assert_eq!(failure["error"]["code"], "invalid_input");
            assert_eq!(request, before);
        }
    }

    #[test]
    fn integer_budgets_reject_zero_fraction_negative_boolean_and_string() {
        for value in [json!(0), json!(1.5), json!(-1), json!(true), json!("10")] {
            let mut request = json!({"schema":"qspquest.run/v0.1","budgets":{"events":value}});
            let failure = admit(&mut request, Profile::Local)
                .unwrap_err()
                .into_value();
            assert_eq!(failure["error"]["expression"], "/budgets/events");
        }
    }

    #[test]
    fn profile_boundaries_and_population_aggregate_remain_distinct() {
        for (profile, subjects, observations, iterations, evaluations) in [
            (Profile::Hosted, 16, 256, 300, 2000),
            (Profile::Local, 128, 10_000, 10_000, 100_000),
        ] {
            let mut request = json!({"subjects":vec![json!({"observations":[]});subjects],"max_iterations":iterations,"max_evaluations":evaluations});
            request["subjects"][0]["observations"] = json!(vec![json!({}); observations]);
            admit(&mut request, profile).unwrap();
            request["subjects"][1]["observations"] = json!([{}]);
            assert_eq!(
                admit(&mut request, profile).unwrap_err().into_value()["error"]["expression"],
                "/subjects"
            );
            let mut too_many = json!({"subjects":vec![json!({"observations":[]});subjects+1]});
            assert!(admit(&mut too_many, profile).is_err());
            for (field, limit) in [
                ("max_iterations", iterations),
                ("max_evaluations", evaluations),
            ] {
                let mut request = json!({field:limit+1});
                assert!(admit(&mut request, profile).is_err());
            }
        }
    }

    #[test]
    fn local_two_effect_saem_admits_the_native_ceiling_without_changing_other_limits() {
        let base = json!({"schema":"qspquest.fit/v0.1", "problem":{"kind":"saem",
            "request":{"random_effects":[{"parameter":"a"},{"parameter":"b"}],
            "subjects":[{"observations":[]}],"max_evaluations":200_000}}});
        let mut request = base.clone();
        admit(&mut request, Profile::Local).unwrap();
        assert_eq!(request, base);
        let mut nested = json!({"fit_request":base.clone()});
        admit(&mut nested, Profile::Local).unwrap();
        let mut three = base.clone();
        three["problem"]["request"]["random_effects"] = json!(vec![json!({}); 3]);
        admit(&mut three, Profile::Local).unwrap();
        for (kind, effects, maximum) in [
            ("individual", 2, 200_000),
            ("focei", 2, 200_000),
            ("saem", 1, 200_000),
            ("saem", 2, 200_001),
            ("saem", 9, 200_000),
        ] {
            let mut request = base.clone();
            request["problem"]["kind"] = json!(kind);
            request["problem"]["request"]["random_effects"] = json!(vec![json!({}); effects]);
            request["problem"]["request"]["max_evaluations"] = json!(maximum);
            let before = request.clone();
            let error = admit(&mut request, Profile::Local)
                .unwrap_err()
                .into_value();
            assert_eq!(
                error["error"]["expression"],
                "/problem/request/max_evaluations"
            );
            assert_eq!(request, before);
        }
        let mut hosted = base;
        let error = admit(&mut hosted, Profile::Hosted)
            .unwrap_err()
            .into_value();
        assert_eq!(error["error"]["code"], "budget");
        assert_eq!(
            error["error"]["expression"],
            "/problem/request/max_evaluations"
        );
    }

    #[test]
    fn scientific_names_are_not_workload_keys_and_are_unchanged() {
        let mut request = json!({"schema":"qspquest.run/v0.1","parameters":{"max_iterations":{"value":3,"unit":"1"},"subjects":{"value":4,"unit":"1"}},"covariates":{"random_effects":{"value":2,"unit":"1"}},"initial_states":{"total_budgets":{"value":0,"unit":"mg"}}});
        let before = request.clone();
        admit(&mut request, Profile::Hosted).unwrap();
        for field in ["parameters", "covariates", "initial_states"] {
            assert_eq!(request[field], before[field]);
        }
    }

    #[test]
    fn withheld_solver_and_effect_limits_are_explicit() {
        for solver in ["diffsol_rosenbrock23", "diffsol_rodas5p"] {
            let mut request = json!({"nested":{"schema":"qspquest.run/v0.1","solver":solver}});
            let failure = admit(&mut request, Profile::Local)
                .unwrap_err()
                .into_value();
            assert_eq!(failure["error"]["code"], "unsupported_solver");
            assert_eq!(failure["error"]["expression"], "/nested/solver");
        }
        // SAEM fits up to eight random effects; the engine checks each method.
        let mut admitted = json!({"random_effects":vec![json!({}); 8]});
        admit(&mut admitted, Profile::Local).unwrap();
        let mut effects = json!({"random_effects":vec![json!({}); 9]});
        assert_eq!(
            admit(&mut effects, Profile::Local)
                .unwrap_err()
                .into_value()["error"]["code"],
            "unsupported"
        );
    }

    #[test]
    fn pointers_escape_arbitrary_enclosing_names_and_input_limit_does_not_mutate() {
        let mut request = json!({"a/b~c":{"max_evaluations":2001}});
        assert_eq!(
            admit(&mut request, Profile::Hosted)
                .unwrap_err()
                .into_value()["error"]["expression"],
            "/a~1b~0c/max_evaluations"
        );
        let mut large =
            json!({"schema":"qspquest.run/v0.1","text":"x".repeat(crate::policy::INPUT_LIMIT)});
        let before = large.clone();
        assert_eq!(
            admit(&mut large, Profile::Local).unwrap_err().into_value()["error"]["code"],
            "input_limit"
        );
        assert_eq!(large, before);
    }

    #[test]
    fn defaults_cannot_push_effective_input_beyond_the_byte_limit() {
        let mut request = json!({"schema":"qspquest.run/v0.1","text":""});
        let overhead = request.to_string().len();
        request["text"] = json!("x".repeat(crate::policy::INPUT_LIMIT - overhead - 1));
        assert_eq!(request.to_string().len(), crate::policy::INPUT_LIMIT - 1);
        let before = request.clone();
        assert_eq!(
            admit(&mut request, Profile::Hosted)
                .unwrap_err()
                .into_value()["error"]["code"],
            "input_limit"
        );
        assert_eq!(request, before);
    }

    #[test]
    fn public_admission_retains_the_pointer_in_the_existing_closed_contract() {
        let mut request = json!({"problem":{"subjects":[{"observations":false}]}});
        let before = request.clone();
        let mut result = crate::policy::admit(&mut request, true).unwrap_err();
        crate::contract::attach(&mut result, "synthetic-fixture", false);
        let failure: crate::contract::Failure =
            serde_json::from_value(result["contract"]["failure"].clone()).unwrap();
        assert_eq!(failure.code, "invalid_input");
        assert_eq!(
            failure.expression.as_deref(),
            Some("/problem/subjects/0/observations")
        );
        assert_eq!(request, before);
        assert_eq!(result["contract"]["status"], "request_failure");
    }
}
