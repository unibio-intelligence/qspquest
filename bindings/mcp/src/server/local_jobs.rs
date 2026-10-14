//! Explicit local tools use the same typed preparation and bounded compute as synchronous calls.
use super::*;
use crate::local_jobs::{self, CompletedUnit, Failure, Operation, State, Ticket};

fn failure(value: &Value) -> Failure {
    Failure {
        code: value["error"]["code"]
            .as_str()
            .unwrap_or("local_job_failed")
            .chars()
            .take(128)
            .collect(),
        message: value["error"]["message"]
            .as_str()
            .unwrap_or("Inspect the completed unit artifact and its scientific status")
            .chars()
            .take(512)
            .collect(),
    }
}

impl Server {
    pub fn with_local_jobs(mut self, path: &std::path::Path) -> Result<Self, String> {
        if self.hosted
            || self.deployment_profile != crate::deployment::Profile::Local
            || self.artifact_store.is_none()
        {
            return Err("Local jobs require local stdio and --artifact-dir PATH; use distinct dedicated private directories for jobs and results".into());
        }
        self.local_jobs = Some(Arc::new(
            local_jobs::Store::open(path).map_err(|value| failure(&value).message)?,
        ));
        Ok(self)
    }
    pub(super) fn local_job_store(&self) -> Result<Arc<local_jobs::Store>, Value> {
        if self.hosted || self.deployment_profile != crate::deployment::Profile::Local {
            return Err(policy::error(
                "local_jobs_unavailable",
                "Use a local stdio qspquest-mcp with --jobs-dir PATH and --artifact-dir PATH in distinct private directories. This deployment cannot create or charge a local job.",
            ));
        }
        self.local_jobs.clone().ok_or_else(|| policy::error("local_jobs_unconfigured",
            "Restart local stdio qspquest-mcp with --jobs-dir PATH and --artifact-dir PATH in distinct dedicated private directories"))
    }
    pub fn start_local_jobs(&self) -> Result<(), String> {
        if let Some(store) = &self.local_jobs {
            self.local_job_store()
                .map_err(|value| failure(&value).message)?;
            for id in store.queued().map_err(|value| failure(&value).message)? {
                self.spawn_local_job(id);
            }
        }
        Ok(())
    }
    fn validate_local_unit(&self, unit: &local_jobs::Unit) -> Result<(), Value> {
        fn typed<T: serde::de::DeserializeOwned>(value: &Value) -> Result<(), Value> {
            serde_json::from_value::<T>(value.clone())
                .map(|_| ())
                .map_err(|error| policy::error("invalid_tool_arguments", error.to_string()))
        }
        crate::core::time_validation::finite_inputs(&unit.arguments, "arguments")?;
        match unit.operation {
            Operation::Simulate => typed::<Simulate>(&unit.arguments),
            Operation::ScanParameters => typed::<Scan>(&unit.arguments),
            Operation::FitScalar => typed::<ScalarFit>(&unit.arguments),
            Operation::FitIndividual => typed::<IndividualFit>(&unit.arguments),
            Operation::FitPopulation => typed::<PopulationFit>(&unit.arguments),
            #[cfg(feature = "analysis")]
            Operation::SteadyState => typed::<NativeAnalysis>(&unit.arguments),
            #[cfg(feature = "analysis")]
            Operation::MorrisScreening => typed::<MorrisAnalysis>(&unit.arguments),
            #[cfg(not(feature = "analysis"))]
            Operation::SteadyState | Operation::MorrisScreening => Err(policy::error(
                "local_job_operation",
                "This work unit requires an analysis feature build",
            )),
        }
    }
    pub(super) async fn submit_local_work(
        &self,
        input: local_jobs::Submit,
    ) -> Result<Value, Value> {
        let store = self.local_job_store()?;
        let (id, digest) = local_jobs::submitted(&input)?;
        if let Some(job) = store.lookup(&id, &digest)? {
            return Ok(json!({"local_job":job}));
        }
        let mut tickets = Vec::with_capacity(input.work.len());
        // Admit every unit before publishing a job. Preparation does no solve,
        // likelihood evaluation, optimization or simulation random draws.
        for unit in input.work {
            self.validate_local_unit(&unit)?;
            let operation = unit.operation.name();
            let prepared = self
                .prepare_invocation(operation, unit.arguments.clone())
                .await?;
            let plan = self.describe_invocation(operation, &prepared).await?;
            if let Some(expected) = unit
                .arguments
                .get("expected_plan_hash")
                .filter(|v| !v.is_null())
            {
                if expected.as_str() != Some(plan.plan_hash.as_str()) {
                    return Err(policy::error(
                        "plan_mismatch",
                        "Work unit no longer matches expected_plan_hash; inspect plan_run for the same arguments before submitting",
                    ));
                }
            }
            tickets.push(Ticket {
                unit,
                plan_hash: plan.plan_hash,
            });
        }
        let (job, dispatch) = store.enqueue(id.clone(), digest, tickets)?;
        if dispatch {
            self.spawn_local_job(id);
        }
        Ok(json!({"local_job":job}))
    }
    fn verify_local_prefix(&self, job: &local_jobs::Descriptor) -> Result<(), Value> {
        let store = self.artifact_store.as_ref().ok_or_else(|| {
            policy::error(
                "local_job_artifacts",
                "Local jobs require the original result artifact directory",
            )
        })?;
        for unit in &job.completed {
            let expected = unit.artifact.as_ref().ok_or_else(|| policy::error(
                "local_job_artifacts", "A committed unit has no retained artifact; start corrected work with a new key"))?;
            let id = expected
                .uri
                .strip_prefix(crate::artifacts::URI_PREFIX)
                .ok_or_else(|| {
                    policy::error("local_job_artifacts", "Invalid committed artifact URI")
                })?;
            let actual = store
                .descriptor(id)
                .map_err(|error| policy::error(error.code, error.message))?;
            if serde_json::to_value(actual).unwrap() != serde_json::to_value(expected).unwrap() {
                return Err(policy::error(
                    "local_job_artifacts",
                    "Committed artifact identity differs; no unit was replayed",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn resume_local_work(&self, input: local_jobs::Resume) -> Result<Value, Value> {
        let store = self.local_job_store()?;
        let job = store.get(&input.job_id)?;
        if job.retryable {
            self.verify_local_prefix(&job)?;
        }
        let (job, dispatch) = store.resume(&input.job_id, input.retry_uncommitted)?;
        if dispatch {
            self.spawn_local_job(input.job_id);
        }
        Ok(json!({"local_job":job}))
    }
    fn spawn_local_job(&self, id: String) {
        let server = self.clone();
        tokio::spawn(async move {
            let Ok(_slot) = server.local_job_slots.clone().acquire_owned().await else {
                return;
            };
            let Some(store) = server.local_jobs.clone() else {
                return;
            };
            match store.begin(&id) {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    eprintln!("Local job admission failed: {}", failure(&error).message);
                    return;
                }
            }
            if let Err(error) = server.drive_local_job(&id, &store).await {
                if let Err(storage_error) = store.finish(&id, Some(failure(&error))) {
                    eprintln!(
                        "Local job journal failed: {}",
                        failure(&storage_error).message
                    );
                }
            }
        });
    }
    async fn drive_local_job(&self, id: &str, store: &local_jobs::Store) -> Result<(), Value> {
        let initial = store.get(id)?;
        self.verify_local_prefix(&initial)?;
        let tickets = store.tickets(id)?;
        for (index, ticket) in tickets
            .into_iter()
            .enumerate()
            .skip(initial.completed.len())
        {
            if store.get(id)?.state != State::Running {
                break;
            }
            self.validate_local_unit(&ticket.unit)?;
            let operation = ticket.unit.operation.name();
            let mut prepared = self
                .prepare_invocation(operation, ticket.unit.arguments.clone())
                .await?;
            let plan = self.describe_invocation(operation, &prepared).await?;
            if plan.plan_hash != ticket.plan_hash {
                return Err(policy::error(
                    "local_job_plan_changed",
                    "Source, library, request or preparation changed after admission. Completed units remain retained; inspect new plans and use a new key",
                ));
            }
            prepared.dose_event_receipts = plan.dose_event_receipts;
            prepared.scheduled_boundary_receipt = plan.scheduled_boundary_receipt;
            prepared.initialization_receipt = plan.initialization_receipt;
            if !store.start_unit(id, index)? {
                break;
            }
            let failure_context = json!({"plan_hash":ticket.plan_hash,
                "effective_request_hash":prepared.effective_hash,
                "effective_request":prepared.request,
                "model_provenance":prepared.metadata,
                "model_source_sha256":prepared.original_source_hash,
                "execution_result_available":false});
            // Never drop a running native future on cancellation. Retain its
            // complete result before looking at the cancellation state again.
            let mut result = match self
                .execute_prepared_invocation(operation, prepared, Some(ticket.plan_hash.clone()))
                .await
            {
                Ok(value) => value,
                Err(mut error) => {
                    for (key, value) in failure_context.as_object().unwrap() {
                        error[key] = value.clone();
                    }
                    self.persist_result(operation, &mut error)?;
                    error
                }
            };
            let version = self.library.read().unwrap().version.clone();
            crate::contract::attach(&mut result, &version, false);
            let artifact: crate::artifacts::Descriptor =
                serde_json::from_value(result["result_artifact"].clone())
                    .map_err(|error| policy::error("local_job_artifacts", error.to_string()))?;
            let status = result["contract"]["status"]
                .as_str()
                .unwrap_or("request_error")
                .to_owned();
            let completed = CompletedUnit {
                index,
                label: ticket.unit.label,
                operation: operation.into(),
                plan_hash: ticket.plan_hash,
                scientific_status: status.clone(),
                artifact: Some(artifact),
                failure: (status != "success").then(|| failure(&result)),
            };
            store.finish_unit(id, completed)?;
        }
        store.finish(id, None)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unavailable_deployments_fail_before_admission_or_dispatch() {
        let library = Library::embedded().unwrap();
        for hosted in [false, true] {
            let server = Server::new(library.clone(), hosted, Tier::Free, 1);
            let input = local_jobs::Submit {
                idempotency_key: "not-admitted".into(),
                work: vec![],
            };
            let error = server.submit_local_work(input).await.unwrap_err();
            assert_eq!(
                error["error"]["code"],
                if hosted {
                    "local_jobs_unavailable"
                } else {
                    "local_jobs_unconfigured"
                }
            );
            assert!(server.local_jobs.is_none());
            assert!(server.compiled.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn work_units_obey_the_same_closed_input_contracts_as_synchronous_tools() {
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 1);
        let mut unit = local_jobs::Unit {
            label: "synthetic".into(),
            operation: Operation::Simulate,
            arguments: json!({"model":{"source":"synthetic"}}),
        };
        assert!(server.validate_local_unit(&unit).is_ok());
        unit.arguments["misspelled_outputs"] = json!(["cp"]);
        assert_eq!(
            server.validate_local_unit(&unit).unwrap_err()["error"]["code"],
            "invalid_tool_arguments"
        );
        unit.arguments =
            json!({"model":{"source":"synthetic"},"times":[{"value":true,"unit":"h"}]});
        assert!(server.validate_local_unit(&unit).is_err());
        unit.operation = Operation::FitPopulation;
        assert!(server.validate_local_unit(&unit).is_err());
    }
}
