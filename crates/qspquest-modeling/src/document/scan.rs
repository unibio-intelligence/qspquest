use super::*;
use qspquest_core::{identity::canonical_hash, scan::*, Budgets};

pub(super) struct PreparedScanPoint {
    pub(super) id: String,
    pub(super) run: qspquest_core::run::RunRequest,
    pub(super) prepared: PreparedRun,
}
pub(super) struct PreparedScan {
    request_hash: String,
    pub(super) points: Vec<PreparedScanPoint>,
}

impl CompiledDocument {
    /// Prepare every scan point, without integrating or evaluating readouts.
    /// The returned point requests carry the actual partitioned budgets.
    pub fn plan_scan(
        self: &Arc<Self>,
        request: &ScanRequest,
    ) -> Result<qspquest_core::planning::ScanPlan, Error> {
        let prepared = self.prepare_scan(request)?;
        self.describe_prepared_scan(request, &prepared)
    }
    pub(super) fn describe_prepared_scan(
        &self,
        request: &ScanRequest,
        prepared: &PreparedScan,
    ) -> Result<qspquest_core::planning::ScanPlan, Error> {
        use qspquest_core::planning::*;
        let points = prepared
            .points
            .iter()
            .map(|point| {
                Ok(ScanPointPlan {
                    id: point.id.clone(),
                    parameters: point.run.parameters.clone(),
                    run: self
                        .describe_prepared_run(&point.run, None, &point.prepared)
                        .map_err(|e| scan_point_error(&point.id, e))?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(ScanPlan {
            schema: ScanPlanSchema::V01,
            scan_request_hash: prepared.request_hash.clone(),
            total_budgets: request.total_budgets,
            points,
        })
    }
    /// Evaluate 1..=25 explicit parameter points in request order.
    /// Total budgets are conservatively partitioned before execution.
    pub fn scan(self: &Arc<Self>, request: &ScanRequest) -> Result<ScanResult, Error> {
        let prepared = self.prepare_scan(request)?;
        self.execute_prepared_scan(prepared)
    }
    pub(super) fn execute_prepared_scan(
        self: &Arc<Self>,
        prepared: PreparedScan,
    ) -> Result<ScanResult, Error> {
        let mut results = Vec::with_capacity(prepared.points.len());
        for point in prepared.points {
            let result = self
                .execute_prepared(&point.run, None, point.prepared)
                .map_err(|e| scan_point_error(&point.id, e))?;
            results.push(ScanPointResult {
                id: point.id,
                parameters: point.run.parameters,
                result,
            });
        }
        Ok(ScanResult {
            schema: ScanResultSchema::V01,
            scan_request_hash: prepared.request_hash,
            results,
        })
    }

    pub(super) fn prepare_scan(
        self: &Arc<Self>,
        request: &ScanRequest,
    ) -> Result<PreparedScan, Error> {
        let count = request.points.len();
        if count == 0 || count > 25 {
            return Err(invalid("scan requires 1..25 explicit points"));
        }
        let total = request.total_budgets;
        if total.solver_callbacks > u32::MAX as u64
            || total.events > u32::MAX as usize
            || total.output_values > u32::MAX as usize
        {
            return Err(invalid(
                "scan budgets must fit the shared unsigned 32-bit range",
            ));
        }
        let mut ids = BTreeSet::new();
        for point in &request.points {
            if point.id.is_empty() || point.id.len() > 128 || !ids.insert(&point.id) {
                return Err(invalid(
                    "scan point IDs must be unique and contain 1..128 bytes",
                ));
            }
        }
        let budgets = Budgets {
            solver_callbacks: request
                .run
                .budgets
                .solver_callbacks
                .min(total.solver_callbacks / count as u64),
            events: request.run.budgets.events.min(total.events / count),
            output_values: request
                .run
                .budgets
                .output_values
                .min(total.output_values / count),
        };
        let mut points = Vec::with_capacity(count);
        for point in &request.points {
            let mut run = request.run.clone();
            run.parameters.extend(point.parameters.clone());
            run.budgets = budgets;
            let prepared = self
                .prepare_selected(&run, None)
                .map_err(|e| scan_point_error(&point.id, e))?;
            points.push(PreparedScanPoint {
                id: point.id.clone(),
                run,
                prepared,
            });
        }
        Ok(PreparedScan {
            request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"explicit_parameter_scan_v1","request":request}),
            )?,
            points,
        })
    }
}

fn scan_point_error(id: &str, mut error: Error) -> Error {
    error.expression = Some(match error.expression {
        Some(field) if !field.is_empty() => format!("points.{id}.{field}"),
        _ => format!("points.{id}"),
    });
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture(undefined_readout: bool) -> (Arc<CompiledDocument>, ScanRequest) {
        let mut model: Value = serde_json::from_str(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        model["parameters"].as_array_mut().unwrap().push(json!({
            "name":"delay", "default":{"value":0.5,"unit":"h"},
            "transform":"identity","fixed":true,"description":""
        }));
        model["states"][0]["dosing"]["lag"] = json!({"kind":"symbol","name":"delay"});
        if undefined_readout {
            model["outputs"].as_array_mut().unwrap().push(json!({
                "name":"undefined_readout","unit":"1","expression":{
                    "kind":"binary","operator":"divide","arguments":[
                        {"kind":"literal","value":0},{"kind":"literal","value":0}
                    ]
                }
            }));
        }
        let compiled = CompiledDocument::compile(&serde_json::from_value(model).unwrap()).unwrap();
        let run: qspquest_core::run::RunRequest = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-one-compartment.json"
        ))
        .unwrap();
        let request = ScanRequest {
            schema: ScanSchema::V01,
            total_budgets: run.budgets,
            run,
            points: vec![
                ScanPoint {
                    id: "first".into(),
                    parameters: BTreeMap::new(),
                },
                ScanPoint {
                    id: "later".into(),
                    parameters: BTreeMap::from([(
                        "delay".into(),
                        Quantity {
                            value: 13.,
                            unit: "h".into(),
                        },
                    )]),
                },
            ],
        };
        (compiled, request)
    }

    #[test]
    fn later_point_regimen_is_rejected_before_any_readout() {
        let (compiled, request) = fixture(true);
        let mut first = request.run.clone();
        first
            .parameters
            .extend(request.points[0].parameters.clone());
        assert!(compiled.prepare_selected(&first, None).is_ok());
        assert!(compiled.execute(&first).is_err());
        let prepared = compiled.prepare_scan(&request).err().unwrap();
        assert_eq!(
            prepared.expression.as_deref(),
            Some("points.later.administrations[0]")
        );
        assert_eq!(compiled.scan(&request).unwrap_err(), prepared);
        assert_eq!(compiled.plan_scan(&request).unwrap_err(), prepared);
        let mut valid = request;
        valid.points[1].parameters.clear();
        assert_eq!(compiled.prepare_scan(&valid).unwrap().points.len(), 2);
        assert_eq!(compiled.plan_scan(&valid).unwrap().points.len(), 2);
    }

    #[test]
    fn scan_consumes_prepared_points_and_preserves_identity_and_budget_partition() {
        let (compiled, mut request) = fixture(false);
        request.points[1].parameters.insert(
            "delay".into(),
            Quantity {
                value: 1.,
                unit: "h".into(),
            },
        );
        let prepared = compiled.prepare_scan(&request).unwrap();
        let expected: Vec<_> = prepared
            .points
            .iter()
            .map(|p| compiled.execute(&p.run).unwrap())
            .collect();
        let result = compiled.scan(&request).unwrap();
        let plan = compiled.plan_scan(&request).unwrap();
        assert_eq!(plan.scan_request_hash, result.scan_request_hash);
        for (planned, executed) in plan.points.iter().zip(&result.results) {
            assert_eq!(planned.id, executed.id);
            assert_eq!(planned.run.prospective_identity, executed.result.identity);
            assert_eq!(
                planned.run.workload.budgets.solver_callbacks,
                request.total_budgets.solver_callbacks / 2
            );
        }
        assert_eq!(result.scan_request_hash, prepared.request_hash);
        for ((actual, expected), point) in result.results.iter().zip(expected).zip(prepared.points)
        {
            assert_eq!(actual.id, point.id);
            assert_eq!(actual.result.identity, expected.identity);
            assert_eq!(actual.result.times, expected.times);
            assert_eq!(actual.result.outputs[0].values, expected.outputs[0].values);
            assert_eq!(
                point.run.budgets.solver_callbacks,
                request.total_budgets.solver_callbacks / 2
            );
            assert_eq!(point.run.budgets.events, request.total_budgets.events / 2);
            assert_eq!(
                point.run.budgets.output_values,
                request.total_budgets.output_values / 2
            );
        }
    }
}
