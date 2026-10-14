use super::*;
use qspquest_core::{identity::canonical_hash, morris::*, planning::*};

struct PreparedMorris {
    output: usize,
    bounds: Vec<(f64, f64, Unit)>,
    steps: Vec<(usize, usize, usize, f64)>,
    scan: scan::PreparedScan,
    observation: PlannedObservation,
    analysis_request_hash: String,
}

fn execution_hash(
    request: &MorrisExecutionRequest,
    analysis_request_hash: &str,
) -> Result<String, Error> {
    canonical_hash(
        &serde_json::json!({"request":request,"analysis_request_hash":analysis_request_hash}),
    )
}

impl CompiledDocument {
    fn prepare_morris_execution(
        self: &Arc<Self>,
        request: &MorrisExecutionRequest,
    ) -> Result<
        (
            MorrisRequest,
            PreparedMorris,
            Option<MorrisDesignProvenance>,
        ),
        Error,
    > {
        let (explicit, design) = match &request.problem {
            MorrisProblem::Explicit(r) => (r.clone(), None),
            MorrisProblem::Generated(r) => {
                let generated = generate_morris_design(r)?;
                (
                    generated.request,
                    Some(MorrisDesignProvenance {
                        generator: generated.generator,
                        seed: generated.seed,
                        design_request_hash: generated.design_request_hash,
                    }),
                )
            }
        };
        let prepared = self.prepare_morris(&explicit)?;
        Ok((explicit, prepared, design))
    }

    /// Describe the admitted design and exact point grids without evaluating effects.
    pub fn plan_morris(
        self: &Arc<Self>,
        request: &MorrisExecutionRequest,
    ) -> Result<MorrisPlan, Error> {
        let (explicit, prepared, design) = self.prepare_morris_execution(request)?;
        let scan = self.describe_prepared_scan(&explicit.scan, &prepared.scan)?;
        let ranges = explicit
            .ranges
            .iter()
            .zip(&prepared.bounds)
            .map(|(r, (lower, span, unit))| MorrisRangePlan {
                parameter: r.parameter.clone(),
                lower: *lower,
                upper: lower + span,
                unit: planning::exact_unit(*unit, Some(&r.lower.unit)),
            })
            .collect();
        let steps = prepared
            .steps
            .iter()
            .map(|(a, b, p, delta)| MorrisStepPlan {
                from_point: explicit.scan.points[*a].id.clone(),
                to_point: explicit.scan.points[*b].id.clone(),
                parameter: explicit.ranges[*p].parameter.clone(),
                normalized_delta: *delta,
            })
            .collect();
        Ok(MorrisPlan {
            schema: MorrisPlanSchema::V01,
            request: request.clone(),
            analysis_request_hash: execution_hash(request, &prepared.analysis_request_hash)?,
            design,
            ranges,
            trajectories: explicit.trajectories,
            steps,
            output: PlannedOutput {
                name: explicit.output,
                unit: self.output_units[prepared.output].clone(),
            },
            row: explicit.row,
            observation: prepared.observation,
            scan,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::ReadoutDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::ElementaryEffectOverflow,
                DeferredCheck::IndependentScientificQualification,
            ],
        })
    }

    pub fn execute_morris(
        self: &Arc<Self>,
        request: &MorrisExecutionRequest,
    ) -> Result<MorrisExecutionResult, Error> {
        let (explicit, prepared, design) = self.prepare_morris_execution(request)?;
        let mut result = self.execute_prepared_morris(&explicit, prepared)?;
        result.analysis_request_hash = execution_hash(request, &result.analysis_request_hash)?;
        Ok(MorrisExecutionResult {
            schema: MorrisResultSchema::V01,
            result,
            design,
        })
    }

    pub fn morris(self: &Arc<Self>, request: &MorrisRequest) -> Result<MorrisResult, Error> {
        let prepared = self.prepare_morris(request)?;
        self.execute_prepared_morris(request, prepared)
    }

    fn prepare_morris(self: &Arc<Self>, request: &MorrisRequest) -> Result<PreparedMorris, Error> {
        let d = request.ranges.len();
        let count = request.scan.points.len();
        if d == 0
            || d > 11
            || request.trajectories.len() < 2
            || request.trajectories.len() > 12
            || count > 25
            || request.num_levels < 4
            || request.num_levels > 100
            || request.num_levels % 2 != 0
        {
            return Err(invalid("Morris requires 1..11 parameters, 2..12 trajectories, at most 25 points and an even 4..100-level grid"));
        }
        let output = self
            .output_names
            .iter()
            .position(|n| n == &request.output)
            .ok_or_else(|| invalid("unknown Morris output"))?;
        let mut names = BTreeSet::new();
        let mut bounds = Vec::new();
        for r in &request.ranges {
            if !names.insert(r.parameter.clone())
                || !self.parameters.iter().any(|p| p.name == r.parameter)
            {
                return Err(invalid(
                    "Morris range names must be unique model parameters",
                ));
            }
            let unit = Unit::parse(&r.lower.unit)?;
            let lo = quantity(&r.lower)?.in_unit(unit)?;
            let hi = quantity(&r.upper)?.in_unit(unit)?;
            if !lo.is_finite() || !hi.is_finite() || !(hi - lo).is_finite() || hi <= lo {
                return Err(invalid("Morris ranges require finite increasing bounds"));
            }
            bounds.push((lo, hi - lo, unit));
        }
        let mut points = BTreeMap::new();
        let levels = (request.num_levels - 1) as f64;
        let delta = request.num_levels as f64 / (2. * levels);
        for (index, p) in request.scan.points.iter().enumerate() {
            if p.parameters.keys().cloned().collect::<BTreeSet<_>>() != names {
                return Err(invalid(
                    "each Morris point must specify exactly the selected parameters",
                ));
            }
            let mut x = Vec::new();
            for (r, (lo, span, unit)) in request.ranges.iter().zip(&bounds) {
                let value = (quantity(&p.parameters[&r.parameter])?.in_unit(*unit)? - lo) / span;
                if !value.is_finite()
                    || !(-1e-12..=1. + 1e-12).contains(&value)
                    || (value * levels - (value * levels).round()).abs() > 1e-8
                {
                    return Err(invalid("Morris point is outside its normalized grid"));
                }
                x.push((value * levels).round() / levels);
            }
            if points.insert(p.id.clone(), (index, x)).is_some() {
                return Err(invalid("duplicate Morris point ID"));
            }
        }
        let mut used = BTreeSet::new();
        let mut steps = Vec::new();
        for path in &request.trajectories {
            if path.len() != d + 1 {
                return Err(invalid("Morris trajectory must contain d+1 points"));
            }
            for id in path {
                if !points.contains_key(id) || !used.insert(id) {
                    return Err(invalid(
                        "Morris trajectories require distinct known point IDs",
                    ));
                }
            }
            let mut changed = BTreeSet::new();
            for pair in path.windows(2) {
                let (a, x) = &points[&pair[0]];
                let (b, y) = &points[&pair[1]];
                let changes: Vec<_> = x
                    .iter()
                    .zip(y)
                    .enumerate()
                    .filter_map(|(i, (x, y))| {
                        if (y - x).abs() > 1e-12 {
                            Some((i, y - x))
                        } else {
                            None
                        }
                    })
                    .collect();
                if changes.len() != 1 {
                    return Err(invalid("Morris step must change exactly one parameter"));
                }
                let (p, step) = changes[0];
                if (step.abs() - delta).abs() > 1e-8 || !changed.insert(p) {
                    return Err(invalid(
                        "Morris step size or parameter permutation is invalid",
                    ));
                }
                steps.push((*a, *b, p, step));
            }
        }
        if used.len() != count {
            return Err(invalid(
                "every scan point must belong to a Morris trajectory",
            ));
        }
        let scan = self.prepare_scan(&request.scan)?;
        let mut grid = None;
        for point in &scan.points {
            let driver_rows = Self::planned_driver_rows(&point.prepared);
            let coordinates = self.planned_observations(&point.prepared, &driver_rows)?;
            if grid.as_ref().is_some_and(|first| first != &coordinates) {
                return Err(invalid(
                    "Morris row requires identical time/side grids across points",
                ));
            }
            if request.row >= coordinates.len() {
                let mut error = invalid("Morris output row is out of range");
                error.expression = Some("row".into());
                return Err(error);
            }
            if grid.is_none() {
                grid = Some(coordinates);
            }
        }
        let observation = grid.unwrap()[request.row].clone();
        Ok(PreparedMorris {
            output,
            bounds,
            steps,
            scan,
            observation,
            analysis_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"morris_explicit_grid_v1","request":request}),
            )?,
        })
    }

    fn execute_prepared_morris(
        self: &Arc<Self>,
        request: &MorrisRequest,
        prepared: PreparedMorris,
    ) -> Result<MorrisResult, Error> {
        let PreparedMorris {
            output,
            steps,
            scan,
            analysis_request_hash,
            ..
        } = prepared;
        let d = request.ranges.len();
        let scan = self.execute_prepared_scan(scan)?;
        let first = &scan.results[0].result;
        let mut values = Vec::new();
        for p in &scan.results {
            if p.result.times != first.times || p.result.sides != first.sides {
                return Err(invalid(
                    "Morris row requires identical time/side grids across points",
                ));
            }
            values.push(
                *p.result.outputs[output]
                    .values
                    .get(request.row)
                    .ok_or_else(|| invalid("Morris output row is out of range"))?,
            );
        }
        let mut effects = vec![Vec::new(); d];
        for (a, b, p, delta) in steps {
            let e = (values[b] - values[a]) / delta;
            if !e.is_finite() {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "Morris elementary effect overflow",
                ));
            }
            effects[p].push(e);
        }
        let mut summaries = Vec::new();
        for (range, effects) in request.ranges.iter().zip(effects) {
            let mut mu = 0.;
            let mut m2 = 0.;
            let mut mu_star = 0.;
            for (i, e) in effects.iter().enumerate() {
                let n = (i + 1) as f64;
                let difference = e - mu;
                mu += difference / n;
                m2 += difference * (e - mu);
                mu_star += (e.abs() - mu_star) / n;
            }
            let sigma = libm::sqrt(m2 / (effects.len() - 1) as f64);
            if ![mu, mu_star, sigma].iter().all(|v| v.is_finite()) {
                return Err(Error::new(ErrorCode::Domain, "Morris summary overflow"));
            }
            summaries.push(ElementaryEffects {
                parameter: range.parameter.clone(),
                unit: self.output_units[output].clone(),
                effects,
                mu,
                mu_star,
                sigma,
            });
        }
        Ok(MorrisResult {
            analysis_request_hash,
            scan,
            effects: summaries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture(poison: bool) -> (Value, MorrisExecutionRequest) {
        let mut model: Value = serde_json::from_str(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        if poison {
            model["outputs"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"undefined","unit":"1",
                "expression":{"kind":"binary","operator":"divide","arguments":[
                    {"kind":"literal","value":0},{"kind":"literal","value":0}]}}));
        }
        let request = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-morris.json"
        ))
        .unwrap();
        (model, request)
    }

    #[test]
    fn morris_static_row_and_late_unit_admission_precede_any_readout() {
        let (model, mut request) = fixture(true);
        let compiled = CompiledDocument::compile(&serde_json::from_value(model).unwrap()).unwrap();
        compiled.plan_morris(&request).unwrap();
        assert_eq!(
            compiled.execute_morris(&request).unwrap_err().code,
            ErrorCode::Domain
        );
        let MorrisProblem::Generated(generated) = &request.problem else {
            panic!()
        };
        let mut explicit = generate_morris_design(generated).unwrap().request;
        explicit.row = 10_000;
        request.problem = MorrisProblem::Explicit(explicit.clone());
        for error in [
            compiled.plan_morris(&request).unwrap_err(),
            compiled.execute_morris(&request).unwrap_err(),
        ] {
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.expression.as_deref(), Some("row"));
        }
        explicit.row = 4;
        explicit
            .scan
            .points
            .last_mut()
            .unwrap()
            .parameters
            .get_mut("cl")
            .unwrap()
            .unit = "h".into();
        request.problem = MorrisProblem::Explicit(explicit);
        assert_eq!(
            compiled.plan_morris(&request).unwrap_err().code,
            ErrorCode::Unit
        );
        assert_eq!(
            compiled.execute_morris(&request).unwrap_err().code,
            ErrorCode::Unit
        );
    }

    #[test]
    fn morris_admits_equal_event_side_grids_before_simulating_any_point() {
        let (mut model, mut request) = fixture(true);
        model["parameters"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"delay","default":{"value":0,"unit":"h"},
            "transform":"identity","fixed":true,"description":""}));
        model["states"][0]["dosing"]["lag"] = json!({"kind":"symbol","name":"delay"});
        let MorrisProblem::Generated(generated) = &mut request.problem else {
            panic!()
        };
        generated.ranges[0] = ParameterRange {
            parameter: "delay".into(),
            lower: Quantity {
                value: 0.,
                unit: "h".into(),
            },
            upper: Quantity {
                value: 0.6,
                unit: "h".into(),
            },
        };
        let compiled = CompiledDocument::compile(&serde_json::from_value(model).unwrap()).unwrap();
        for error in [
            compiled.plan_morris(&request).unwrap_err(),
            compiled.execute_morris(&request).unwrap_err(),
        ] {
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert!(
                error.message.contains("identical time/side grids"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn explicit_and_generated_morris_plans_match_execution_and_analytical_effects() {
        let (model, mut request) = fixture(false);
        let compiled = CompiledDocument::compile(&serde_json::from_value(model).unwrap()).unwrap();
        let MorrisProblem::Generated(generated) = &mut request.problem else {
            panic!()
        };
        generated.ranges[0].lower = Quantity {
            value: 100.,
            unit: "mL/h".into(),
        };
        generated.ranges[0].upper = Quantity {
            value: 700.,
            unit: "mL/h".into(),
        };
        for explicit in [false, true] {
            if explicit {
                let MorrisProblem::Generated(generated) = &request.problem else {
                    panic!()
                };
                request.problem =
                    MorrisProblem::Explicit(generate_morris_design(generated).unwrap().request);
            }
            let plan = compiled.plan_morris(&request).unwrap();
            assert_eq!(
                serde_json::to_value(&plan).unwrap(),
                serde_json::to_value(compiled.plan_morris(&request).unwrap()).unwrap()
            );
            assert_eq!(plan.design.is_some(), !explicit);
            assert_eq!((plan.ranges[0].lower, plan.ranges[0].upper), (100., 700.));
            assert_eq!(plan.ranges[0].unit.declared.as_deref(), Some("mL/h"));
            assert_eq!(plan.observation.time, 4.);
            assert_eq!(plan.output.unit, "mg/L");
            assert!(serde_json::to_value(&plan)
                .unwrap()
                .get("effects")
                .is_none());
            let result = compiled.execute_morris(&request).unwrap();
            assert_eq!(
                plan.analysis_request_hash,
                result.result.analysis_request_hash
            );
            assert_eq!(
                plan.scan.scan_request_hash,
                result.result.scan.scan_request_hash
            );
            let mut analytical = BTreeMap::new();
            for (point, run) in plan.scan.points.iter().zip(&result.result.scan.results) {
                assert_eq!(point.run.prospective_identity, run.result.identity);
                assert_eq!(
                    point
                        .run
                        .observations
                        .iter()
                        .map(|p| p.time)
                        .collect::<Vec<_>>(),
                    run.result.times
                );
                let cl = point.run.binding_phases[0]
                    .inputs
                    .iter()
                    .find(|p| p.name == "cl")
                    .unwrap()
                    .value;
                let v = point.run.binding_phases[0]
                    .inputs
                    .iter()
                    .find(|p| p.name == "v")
                    .unwrap()
                    .value;
                analytical.insert(point.id.clone(), 6. / v * libm::exp(-cl / v * 4.));
            }
            for effect in &result.result.effects {
                let expected: Vec<_> = plan
                    .steps
                    .iter()
                    .filter(|step| step.parameter == effect.parameter)
                    .map(|step| {
                        (analytical[&step.to_point] - analytical[&step.from_point])
                            / step.normalized_delta
                    })
                    .collect();
                assert_eq!(effect.effects.len(), expected.len());
                for (actual, expected) in effect.effects.iter().zip(expected) {
                    assert!((actual - expected).abs() < 2e-7, "{actual} != {expected}");
                }
            }
        }
    }
}
