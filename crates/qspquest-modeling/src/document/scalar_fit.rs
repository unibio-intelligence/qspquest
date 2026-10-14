//! Bounded scalar Gaussian fit over the ordinary compiled simulation path.
use super::*;
use qspquest_core::{
    fit::{
        FitStatus, ScalarFitResultSchema, ScalarGaussianFitRequest, ScalarGaussianFitResult,
        ScalarGaussianObjectiveResult,
    },
    identity::canonical_hash,
};

struct PreparedScalarFit {
    lower: f64,
    upper: f64,
    parameter_unit: String,
    data: Vec<(usize, usize, f64, f64)>,
}

fn scalar_error_at(mut error: Error, path: impl Into<String>) -> Error {
    if error.expression.is_none() {
        error.expression = Some(path.into());
    }
    error
}
fn scalar_quantity(value: &Quantity, unit: Unit, path: &str) -> Result<f64, Error> {
    quantity(value)
        .and_then(|value| value.in_unit(unit))
        .map_err(|error| scalar_error_at(error, path))
}

impl CompiledDocument {
    pub fn fit_scalar(
        self: &Arc<Self>,
        request: &ScalarGaussianFitRequest,
    ) -> Result<ScalarGaussianFitResult, Error> {
        let PreparedScalarFit {
            lower,
            upper,
            parameter_unit,
            data,
        } = self.prepare_scalar_fit(request)?;

        let mut run = request.run.clone();
        let mut evaluations = 0usize;
        let mut evaluate = |x: f64| -> Result<(f64, qspquest_core::run::ExecutionIdentity), Error> {
            run.parameters.insert(
                request.parameter.clone(),
                Quantity {
                    value: x,
                    unit: parameter_unit.clone(),
                },
            );
            let result = self.execute(&run)?;
            evaluations += 1;
            let mut nll = 0.;
            for &(output, row, value, sd) in &data {
                let prediction = result.outputs[output]
                    .values
                    .get(row)
                    .copied()
                    .ok_or_else(|| invalid("scalar fit observation row is outside the run grid"))?;
                let z = (value - prediction) / sd;
                nll += 0.5 * z * z + sd.ln() + 0.9189385332046727;
            }
            if !nll.is_finite() {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "scalar Gaussian objective is nonfinite",
                ));
            }
            Ok((nll, result.identity))
        };

        // Safeguarded parabolic interpolation with golden-section steps.
        let (mut a, mut b) = (lower, upper);
        let mut x = a + 0.3819660112501051 * (b - a);
        let (mut fx, mut best_identity) = evaluate(x)?;
        let (mut w, mut v) = (x, x);
        let (mut fw, mut fv) = (fx, fx);
        let (mut d, mut e): (f64, f64) = (0., 0.);
        let mut status = FitStatus::IterationLimit;
        let mut iterations = 0usize;
        for iteration in 0..request.max_iterations {
            iterations = iteration + 1;
            let midpoint = 0.5 * (a + b);
            let tol = f64::EPSILON.sqrt() * x.abs() + request.absolute_tolerance / 3.;
            let twice_tol = 2. * tol;
            if (x - midpoint).abs() <= twice_tol - 0.5 * (b - a) {
                status = FitStatus::Converged;
                break;
            }
            let mut parabolic = false;
            if e.abs() > tol {
                let r = (x - w) * (fx - fv);
                let mut q = (x - v) * (fx - fw);
                let mut p = (x - v) * q - (x - w) * r;
                q = 2. * (q - r);
                if q > 0. {
                    p = -p;
                } else {
                    q = -q;
                }
                let old_e = e;
                e = d;
                if q > 0. && p.abs() < (0.5 * q * old_e).abs() && p > q * (a - x) && p < q * (b - x)
                {
                    d = p / q;
                    parabolic = true;
                    let candidate = x + d;
                    if candidate - a < twice_tol || b - candidate < twice_tol {
                        d = if midpoint >= x { tol } else { -tol };
                    }
                }
            }
            if !parabolic {
                e = if x < midpoint { b - x } else { a - x };
                d = 0.3819660112501051 * e;
            }
            let u = x + if d.abs() >= tol { d } else { tol.copysign(d) };
            let (fu, identity) = evaluate(u)?;
            if fu <= fx {
                if u < x {
                    b = x;
                } else {
                    a = x;
                }
                v = w;
                fv = fw;
                w = x;
                fw = fx;
                x = u;
                fx = fu;
                best_identity = identity;
            } else {
                if u < x {
                    a = u;
                } else {
                    b = u;
                }
                if fu <= fw || w == x {
                    v = w;
                    fv = fw;
                    w = u;
                    fw = fu;
                } else if fu <= fv || v == x || v == w {
                    v = u;
                    fv = fu;
                }
            }
        }
        Ok(ScalarGaussianFitResult {
            schema: ScalarFitResultSchema::V01,
            status,
            parameters: BTreeMap::from([(
                request.parameter.clone(),
                Quantity {
                    value: x,
                    unit: parameter_unit.clone(),
                },
            )]),
            objective: ScalarGaussianObjectiveResult {
                identity: best_identity,
                negative_log_likelihood: fx,
                objective: fx,
                observations: data.len(),
            },
            evaluations,
            iterations,
            fit_request_hash: canonical_hash(&serde_json::json!(request))?,
        })
    }
    /// Describe scalar-fit admission; optimization and future row/domain checks remain deferred.
    pub fn plan_scalar_fit(
        self: &Arc<Self>,
        request: &ScalarGaussianFitRequest,
    ) -> Result<qspquest_core::planning::ScalarFitPlan, Error> {
        use qspquest_core::planning::*;
        let limit = 1_000_000usize;
        let mut items = 0usize;
        let mut admission = None;
        let mut describe = |prepared: &PreparedRun| {
            let r = &prepared.regimen;
            items = [self.state_count, self.output_names.len(), 2, r.protocol.samples.len().saturating_mul(3),
                r.protocol.events.len().saturating_mul(3), r.model.discontinuities().len().saturating_mul(3),
                r.result_rows.saturating_mul(3),
                r.result_rows.saturating_mul(r.model.predicate_points.len() + r.model.dose_predicates.len()), request.observations.len().saturating_mul(2),
                (r.model.bindings.len()+1).saturating_mul(self.parameters.len()+self.covariates.len()+self.individual.len())]
                .into_iter().try_fold(0usize, |sum,n| sum.checked_add(n)).filter(|n| *n <= limit)
                .ok_or_else(|| Error::new(ErrorCode::OutputBudget, "scalar fitting-plan metadata exceeds 1000000 descriptive records; reduce the observation grid"))?;
            admission = Some(self.describe_prepared_run(&request.run, None, prepared)?);
            Ok(())
        };
        let p = self.prepare_scalar_fit_observed(request, Some(&mut describe))?;
        let admission = admission.expect("validated scalar preparation calls its observer");
        let observations = p
            .data
            .iter()
            .map(|&(output, row, value, sd)| GaussianObservationPlan {
                row,
                output: self.output_names[output].clone(),
                unit: self.output_units[output].clone(),
                value,
                additive_sd: sd,
                proportional_sd: 0.,
                model: qspquest_core::fit::GaussianErrorModel::Normal,
                censoring: None,
            })
            .collect();
        let mut plan = ScalarFitPlan {
            schema: ScalarFitPlanSchema::V01,
            request: request.clone(),
            fit_input_hash: canonical_hash(
                &serde_json::json!({"request":request,"admission_identity":admission.prospective_identity}),
            )?,
            plan_hash: String::new(),
            parameter: request.parameter.clone(),
            unit: super::planning::exact_unit(
                Unit::parse(&p.parameter_unit)?,
                Some(&p.parameter_unit),
            ),
            lower: p.lower,
            upper: p.upper,
            first_trial: p.lower + 0.3819660112501051 * (p.upper - p.lower),
            admission,
            observations,
            observation_row_checks_deferred: request.run.regimen.observations.is_none(),
            max_iterations: request.max_iterations,
            absolute_tolerance: request.absolute_tolerance,
            metadata_item_limit: limit,
            metadata_items: items,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::ReadoutDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::Optimization,
                DeferredCheck::Identifiability,
                DeferredCheck::ObservedCovariance,
                DeferredCheck::IndependentScientificQualification,
            ],
        };
        plan.plan_hash =
            canonical_hash(&serde_json::to_value(&plan).map_err(|e| invalid(e.to_string()))?)?;
        Ok(plan)
    }
    fn prepare_scalar_fit(
        self: &Arc<Self>,
        request: &ScalarGaussianFitRequest,
    ) -> Result<PreparedScalarFit, Error> {
        self.prepare_scalar_fit_observed(request, None)
    }
    fn prepare_scalar_fit_observed(
        self: &Arc<Self>,
        request: &ScalarGaussianFitRequest,
        mut observer: Option<&mut dyn FnMut(&PreparedRun) -> Result<(), Error>>,
    ) -> Result<PreparedScalarFit, Error> {
        if request.max_iterations == 0
            || request.max_iterations > 1000
            || !request.absolute_tolerance.is_finite()
            || request.absolute_tolerance <= 0.
            || request.observations.is_empty()
            || request.observations.len() > 500
        {
            let path = if request.max_iterations == 0 || request.max_iterations > 1000 {
                "/max_iterations"
            } else if !request.absolute_tolerance.is_finite() || request.absolute_tolerance <= 0. {
                "/absolute_tolerance"
            } else {
                "/observations"
            };
            return Err(scalar_error_at(
                invalid("scalar fit requires bounded iterations, tolerance and observations"),
                path,
            ));
        }
        let parameter = self
            .parameters
            .iter()
            .find(|p| p.name == request.parameter)
            .ok_or_else(|| {
                scalar_error_at(
                    invalid("scalar fit parameter is not declared"),
                    "/parameter",
                )
            })?;
        if parameter.fixed {
            return Err(scalar_error_at(
                invalid("cannot fit a fixed parameter"),
                "/parameter",
            ));
        }
        let unit = Unit::parse(&parameter.default.unit)
            .map_err(|error| scalar_error_at(error, "/parameter"))?;
        let lower = scalar_quantity(&request.lower, unit, "/lower")?;
        let upper = scalar_quantity(&request.upper, unit, "/upper")?;
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return Err(scalar_error_at(
                invalid("scalar fit bounds must be finite and increasing"),
                "/upper",
            ));
        }
        if let Some(bounds) = &parameter.bounds {
            let below = lower < scalar_quantity(&bounds[0], unit, "/parameter")?;
            if below || upper > scalar_quantity(&bounds[1], unit, "/parameter")? {
                return Err(scalar_error_at(
                    invalid("scalar fit box exceeds declared model bounds"),
                    if below { "/lower" } else { "/upper" },
                ));
            }
        }
        let initial = request
            .run
            .parameters
            .get(&request.parameter)
            .unwrap_or(&parameter.default);
        let initial_path = if request.run.parameters.contains_key(&request.parameter) {
            format!(
                "/run/parameters/{}",
                request.parameter.replace('~', "~0").replace('/', "~1")
            )
        } else {
            "/parameter".to_owned()
        };
        let initial = scalar_quantity(initial, unit, &initial_path)?;
        if !initial.is_finite() || initial < lower || initial > upper {
            return Err(scalar_error_at(
                invalid("scalar fit initial value must lie inside its bounds"),
                initial_path,
            ));
        }

        let mut data = Vec::with_capacity(request.observations.len());
        for (index, observation) in request.observations.iter().enumerate() {
            let path = format!("/observations/{index}");
            let output = self
                .output_names
                .iter()
                .position(|name| name == &observation.output)
                .ok_or_else(|| {
                    scalar_error_at(
                        invalid("scalar fit observation names an unknown output"),
                        format!("{path}/output"),
                    )
                })?;
            let output_unit = Unit::parse(&self.output_units[output])
                .map_err(|error| scalar_error_at(error, format!("{path}/output")))?;
            let value = scalar_quantity(&observation.value, output_unit, &format!("{path}/value"))?;
            if observation.error.model != qspquest_core::fit::GaussianErrorModel::Normal {
                return Err(scalar_error_at(
                    Error::new(
                        ErrorCode::Unsupported,
                        "scalar fit supports normal additive error",
                    ),
                    format!("{path}/error/model"),
                ));
            }
            if observation.censoring.is_some() {
                return Err(scalar_error_at(
                    Error::new(
                        ErrorCode::Unsupported,
                        "scalar fit censored observations are not implemented",
                    ),
                    format!("{path}/censoring"),
                ));
            }
            let sd = scalar_quantity(
                &observation.error.additive_sd,
                output_unit,
                &format!("{path}/error/additive_sd"),
            )?;
            if !value.is_finite()
                || !sd.is_finite()
                || sd <= 0.
                || observation.error.proportional_sd != 0.
            {
                let field = if !value.is_finite() {
                    "value"
                } else if !sd.is_finite() || sd <= 0. {
                    "error/additive_sd"
                } else {
                    "error/proportional_sd"
                };
                return Err(scalar_error_at(invalid(format!(
                    "scalar fit observation {index} requires a finite value and positive additive SD"
                )), format!("{path}/{field}")));
            }
            data.push((output, observation.row, value, sd));
        }

        let prepared = self
            .prepare_selected(&request.run, None)
            .map_err(|error| scalar_error_at(error, "/run"))?;
        // Explicit observation coordinates fix fit rows independently of fitted dosing lags.
        // An implicit trajectory grid can change with fitted event timing; each trial
        // retains its execution-time row check.
        if request.run.regimen.observations.is_some() {
            if let Some(index) = data
                .iter()
                .position(|(_, row, _, _)| *row >= prepared.regimen.result_rows)
            {
                return Err(scalar_error_at(
                    invalid("scalar fit observation row is outside the run grid"),
                    format!("/observations/{index}/row"),
                ));
            }
        }
        if let Some(observer) = observer.as_mut() {
            observer(&prepared)?;
        }
        Ok(PreparedScalarFit {
            lower,
            upper,
            parameter_unit: parameter.default.unit.clone(),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_fit_rows_are_checked_without_evaluating_readouts() {
        let mut document: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        document["outputs"][0]["unit"] = json!("1");
        document["outputs"][0]["expression"] = json!({
            "kind":"binary", "operator":"divide", "arguments":[
                {"kind":"literal", "value":0}, {"kind":"literal", "value":0}
            ]
        });
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let run: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-one-compartment.json"
        ))
        .unwrap();
        let mut request: ScalarGaussianFitRequest = serde_json::from_value(json!({
            "schema":"qspquest.scalar-fit/v0.1", "run":run, "parameter":"cl",
            "lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1,"unit":"L/h"},
            "observations":[{"row":0,"output":"cp","value":{"value":1,"unit":"1"},
                "error":{"model":"normal","additive_sd":{"value":0.1,"unit":"1"},"proportional_sd":0}}],
            "max_iterations":10,"absolute_tolerance":0.0001
        })).unwrap();
        assert!(compiled.prepare_scalar_fit(&request).is_ok());
        let plan = compiled.plan_scalar_fit(&request).unwrap();
        assert_eq!(plan.lower, 0.1);
        assert_eq!(plan.upper, 1.);
        assert_eq!(plan.observations[0].additive_sd, 0.1);
        assert!(!plan.observation_row_checks_deferred);
        let mut implicit = request.clone();
        implicit.run.regimen.observations = None;
        assert!(
            compiled
                .plan_scalar_fit(&implicit)
                .unwrap()
                .observation_row_checks_deferred
        );
        assert_eq!(plan.first_trial, 0.1 + 0.3819660112501051 * 0.9);
        assert!(serde_json::to_value(plan)
            .unwrap()
            .get("objective")
            .is_none());
        assert_eq!(
            compiled.fit_scalar(&request).unwrap_err().code,
            ErrorCode::Domain
        );
        request.observations[0].error.model = qspquest_core::fit::GaussianErrorModel::Lognormal;
        assert_eq!(
            compiled.plan_scalar_fit(&request).unwrap_err(),
            compiled.fit_scalar(&request).unwrap_err()
        );
        assert_eq!(
            compiled.prepare_scalar_fit(&request).err().unwrap().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            compiled.fit_scalar(&request).unwrap_err().code,
            ErrorCode::Unsupported
        );
        request.observations[0].error.model = qspquest_core::fit::GaussianErrorModel::Normal;
        request.observations[0].row = 999;
        let error = compiled.prepare_scalar_fit(&request).err().unwrap();
        assert!(error.message.contains("outside the run grid"));
        assert_eq!(error, compiled.fit_scalar(&request).unwrap_err());
    }
    fn diagnostic_case() -> (Arc<CompiledDocument>, ScalarGaussianFitRequest) {
        let mut document: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        document["outputs"][0]["unit"] = json!("1");
        document["outputs"][0]["expression"] = json!({"kind":"binary","operator":"divide",
            "arguments":[{"kind":"literal","value":0},{"kind":"literal","value":0}]});
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let run: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-one-compartment.json"
        ))
        .unwrap();
        let request = serde_json::from_value(json!({"schema":"qspquest.scalar-fit/v0.1",
            "run":run,"parameter":"cl","lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1,"unit":"L/h"},
            "observations":[{"row":0,"output":"cp","value":{"value":1,"unit":"1"},
                "error":{"additive_sd":{"value":0.1,"unit":"1"},"proportional_sd":0}}],
            "max_iterations":10,"absolute_tolerance":1e-4})).unwrap();
        (compiled, request)
    }
    #[test]
    fn dimensional_refusals_locate_bounds_and_explicit_initial_parameter() {
        let (compiled, request) = diagnostic_case();
        for (slot, pointer) in [
            ("lower", "/lower"),
            ("upper", "/upper"),
            ("initial", "/run/parameters/cl"),
        ] {
            let mut invalid = request.clone();
            match slot {
                "lower" => invalid.lower.unit = "mg".into(),
                "upper" => invalid.upper.unit = "mg".into(),
                _ => {
                    invalid.run.parameters.insert(
                        "cl".into(),
                        Quantity {
                            value: 0.3,
                            unit: "mg".into(),
                        },
                    );
                }
            }
            let error = compiled.plan_scalar_fit(&invalid).unwrap_err();
            assert_eq!(error.code, ErrorCode::Unit);
            assert_eq!(error.expression.as_deref(), Some(pointer));
            assert_eq!(error, compiled.fit_scalar(&invalid).unwrap_err());
        }
    }
    #[test]
    fn native_control_and_selection_errors_preserve_codes_and_locate_fields() {
        let (compiled, request) = diagnostic_case();
        for (slot, pointer) in [
            ("iterations", "/max_iterations"),
            ("tolerance", "/absolute_tolerance"),
            ("observations", "/observations"),
            ("parameter", "/parameter"),
        ] {
            let mut invalid = request.clone();
            match slot {
                "iterations" => invalid.max_iterations = 0,
                "tolerance" => invalid.absolute_tolerance = 0.0,
                "observations" => invalid.observations.clear(),
                _ => invalid.parameter = "missing".into(),
            }
            let error = compiled.plan_scalar_fit(&invalid).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.expression.as_deref(), Some(pointer));
            assert_eq!(error, compiled.fit_scalar(&invalid).unwrap_err());
        }
    }
    #[test]
    fn native_observation_refusals_retain_specific_statistical_and_row_context() {
        let (compiled, request) = diagnostic_case();
        for (slot, pointer, code) in [
            ("output", "/observations/0/output", ErrorCode::InvalidInput),
            ("value", "/observations/0/value", ErrorCode::Unit),
            (
                "sd_unit",
                "/observations/0/error/additive_sd",
                ErrorCode::Unit,
            ),
            (
                "sd_zero",
                "/observations/0/error/additive_sd",
                ErrorCode::InvalidInput,
            ),
            (
                "proportional",
                "/observations/0/error/proportional_sd",
                ErrorCode::InvalidInput,
            ),
            (
                "model",
                "/observations/0/error/model",
                ErrorCode::Unsupported,
            ),
            (
                "censoring",
                "/observations/0/censoring",
                ErrorCode::Unsupported,
            ),
            ("row", "/observations/0/row", ErrorCode::InvalidInput),
        ] {
            let mut invalid = request.clone();
            let observation = &mut invalid.observations[0];
            match slot {
                "output" => observation.output = "missing".into(),
                "value" => observation.value.unit = "mg".into(),
                "sd_unit" => observation.error.additive_sd.unit = "mg".into(),
                "sd_zero" => observation.error.additive_sd.value = 0.0,
                "proportional" => observation.error.proportional_sd = 0.1,
                "model" => {
                    observation.error.model = qspquest_core::fit::GaussianErrorModel::Lognormal
                }
                "censoring" => {
                    observation.censoring = Some(qspquest_core::fit::GaussianCensoring::Left)
                }
                _ => observation.row = 999,
            }
            let error = compiled.plan_scalar_fit(&invalid).unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.expression.as_deref(), Some(pointer));
            assert_eq!(error, compiled.fit_scalar(&invalid).unwrap_err());
        }
    }
    #[test]
    fn scalar_run_boundary_preserves_more_specific_native_initial_state_context() {
        let (compiled, mut request) = diagnostic_case();
        request.run.initial_states.insert(
            "central".into(),
            Quantity {
                value: 1.0,
                unit: "h".into(),
            },
        );
        let error = compiled.plan_scalar_fit(&request).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unit);
        assert_eq!(error.expression.as_deref(), Some("initial_states.central"));
        assert_eq!(error, compiled.fit_scalar(&request).unwrap_err());
    }
}
