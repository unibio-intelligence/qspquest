use super::*;
use qspquest_core::{
    identity::{canonical_hash, SPEC_VERSION},
    regimen::ObservationSide,
    run::*,
};
pub(super) struct PreparedSensitivityRun {
    pub(super) bound: BoundSensitivityDocument,
    pub(super) regimen: super::regimen::PreparedSensitivityRegimen,
}

impl PreparedSensitivityRun {
    pub(super) fn result_rows(&self) -> usize {
        self.regimen.result_rows()
    }
}

impl CompiledSensitivityDocument {
    pub fn run(self: &Arc<Self>, request: &SensitivityRequest) -> Result<SensitivityResult, Error> {
        let prepared = self.prepare_run(request)?;
        self.execute_prepared_run(request, prepared)
    }
    pub(super) fn prepare_run(
        self: &Arc<Self>,
        request: &SensitivityRequest,
    ) -> Result<PreparedSensitivityRun, Error> {
        let run = &request.run;
        if !run.accuracy_control.is_local() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "sampled accuracy control currently supports primal simulation requests",
            ));
        }
        if request.with_respect_to != self.parameters() {
            return Err(invalid(
                "sensitivity parameter order differs from compiled document",
            ));
        }
        if !matches!(run.solver, Solver::DiffsolBdf | Solver::DiffsolRodas5p) || run.seed.is_some()
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "sensitivity runs require deterministic diffsol_bdf or diffsol_rodas5p",
            ));
        }
        if run
            .request_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err(invalid("request_id must contain 1..128 bytes"));
        }
        if run.budgets.solver_callbacks > u32::MAX as u64
            || run.budgets.events > u32::MAX as usize
            || run.budgets.output_values > u32::MAX as usize
        {
            return Err(invalid(
                "budgets must fit the shared native/WASM unsigned 32-bit range",
            ));
        }
        let bound = self.bind_with_initial_states(
            &run.parameters,
            &run.regimen.covariates,
            &run.initial_states,
        )?;
        let regimen = self.prepare_regimen_initial(
            &run.parameters,
            &run.regimen,
            &request.derivative_absolute_tolerances,
            run.budgets,
            &run.initial_states,
        )?;
        let width = self
            .output_names()
            .len()
            .checked_mul(self.parameters().len() + 1)
            .ok_or_else(|| invalid("sensitivity column width overflow"))?;
        if regimen
            .result_rows()
            .checked_mul(width)
            .and_then(|n| n.checked_mul(2))
            .is_none_or(|n| n > run.budgets.output_values)
        {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "sensitivity row and column projection exceeds budget",
            ));
        }
        Ok(PreparedSensitivityRun { bound, regimen })
    }
    pub(super) fn prospective_identity(
        &self,
        request: &SensitivityRequest,
        bound: &BoundSensitivityDocument,
    ) -> Result<ExecutionIdentity, Error> {
        let run = &request.run;
        Ok(ExecutionIdentity {
            model_content_hash: self.primal.model_content_hash.clone(),
            spec_version: SPEC_VERSION.into(),
            compiler_version: concat!("qspquest/", env!("CARGO_PKG_VERSION")).into(),
            compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
            rustc_version: env!("QSPQUEST_RUSTC_VERSION").into(),
            target: env!("QSPQUEST_TARGET").into(),
            build_profile: env!("QSPQUEST_PROFILE").into(),
            rustflags_sha256: env!("QSPQUEST_RUSTFLAGS_SHA256").into(),
            bound_value_hash: canonical_hash(
                &serde_json::json!({"values":bound.primal.values,"initial_states":bound.initial()}),
            )?,
            run_request_hash: canonical_hash(
                &serde_json::json!({"schema":request.schema,"operation":"forward_sensitivity","run_schema":run.schema,"regimen":run.regimen,"solver":run.solver,"budgets":run.budgets,"with_respect_to":request.with_respect_to,"derivative_absolute_tolerances":request.derivative_absolute_tolerances}),
            )?,
            backend: "diffsol".into(),
            backend_version: crate::DIFFSOL_BACKEND_VERSION.into(),
            algorithm: match run.solver {
                Solver::DiffsolRodas5p => "rodas5p_augmented_forward_sensitivity",
                _ => "bdf_augmented_forward_sensitivity",
            }
            .into(),
        })
    }
    pub(super) fn execute_prepared_run(
        self: &Arc<Self>,
        request: &SensitivityRequest,
        prepared: PreparedSensitivityRun,
    ) -> Result<SensitivityResult, Error> {
        let run = &request.run;
        let PreparedSensitivityRun { bound, regimen } = prepared;
        let identity = self.prospective_identity(request, &bound)?;
        let rows = self.simulate_prepared_regimen(regimen, run.solver)?;
        let outputs = self
            .output_names()
            .iter()
            .zip(self.output_units())
            .enumerate()
            .map(|(o, (name, unit))| ResultColumn {
                name: name.clone(),
                unit: unit.clone(),
                values: rows.iter().map(|r| r.values[o]).collect(),
            })
            .collect();
        let mut derivatives = Vec::new();
        for (p, parameter) in self.parameters().iter().enumerate() {
            let parameter_unit = &self
                .primal
                .parameters
                .iter()
                .find(|x| &x.name == parameter)
                .ok_or_else(|| invalid("missing sensitivity parameter"))?
                .default
                .unit;
            for (o, (name, unit)) in self
                .output_names()
                .iter()
                .zip(self.output_units())
                .enumerate()
            {
                let unit = format!("({unit})/({parameter_unit})");
                // Ensure the serialized spelling agrees with runtime metadata.
                Unit::parse(&unit)?.conversion_to(self.derivative_units()[p][o])?;
                derivatives.push(SensitivityColumn {
                    output: name.clone(),
                    parameter: parameter.clone(),
                    unit,
                    values: rows.iter().map(|r| r.derivatives[p][o]).collect(),
                });
            }
        }
        Ok(SensitivityResult {
            schema: SensitivityResultSchema::V01,
            result: RunResult {
                schema: ResultSchema::V01,
                request_id: run.request_id.clone(),
                identity,
                model_id: self.primal.model_id.clone(),
                description: self.primal.description.clone(),
                time_unit: self.primal.time_unit.clone(),
                times: rows.iter().map(|r| r.time).collect(),
                sides: rows
                    .iter()
                    .map(|r| {
                        if r.side == "pre" {
                            ObservationSide::Pre
                        } else {
                            ObservationSide::Post
                        }
                    })
                    .collect(),
                outputs,
            },
            derivatives,
        })
    }
}
