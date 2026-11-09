//! Native and isolated execution dispatch, permits, deadlines and compilation reuse.
use super::*;

/// The WASM sandbox accepts 2 MB of envelope; keep room for its framing.
const SANDBOX_INPUT: usize = 1_900_000;

impl Server {
    pub(super) fn hosted_time_remaining(&self) -> std::time::Duration {
        self.hosted_deadline
            .map_or(policy::HOSTED_DEADLINE, |deadline| {
                deadline.saturating_duration_since(tokio::time::Instant::now())
            })
    }

    pub(super) async fn within_hosted_deadline(
        &self,
        work: impl std::future::Future<Output = Value>,
    ) -> Value {
        if self.hosted {
            tokio::time::timeout(self.hosted_time_remaining(), work)
                .await
                .unwrap_or_else(|_| {
                    policy::error(
                        "deadline",
                        "Hosted request exceeded its aggregate 45-second limit. Active isolated or native work retains its slot until it exits.",
                    )
                })
        } else {
            work.await
        }
    }

    pub(super) async fn compute(
        &self,
        source: String,
        operation: &str,
        request: Value,
        trusted: bool,
    ) -> Result<Value, Value> {
        let version = self.library.read().unwrap().version.clone();
        self.compute_versioned(source, operation, request, trusted, &version)
            .await
    }
    pub(super) async fn compute_versioned(
        &self,
        source: String,
        operation: &str,
        mut request: Value,
        trusted: bool,
        library_version: &str,
    ) -> Result<Value, Value> {
        let deadline = self.hosted.then(|| {
            self.hosted_deadline
                .unwrap_or_else(|| tokio::time::Instant::now() + policy::HOSTED_DEADLINE)
        });
        if deadline.is_some_and(|limit| tokio::time::Instant::now() >= limit) {
            return Err(policy::error(
                "deadline",
                "Hosted request ran out of time before starting computation.",
            ));
        }
        if matches!(operation, "analyze" | "plan_analyze") {
            crate::execution_policy::admit_analysis(
                &mut request,
                crate::execution_policy::Profile::from_hosted(self.hosted),
            )?;
        } else {
            policy::admit(&mut request, self.hosted)?;
        }
        if self.hosted && !trusted && source.len() + request.to_string().len() > SANDBOX_INPUT {
            return Err(policy::error(
                "input_limit",
                "Model source plus request exceed the isolated engine's 1.9 MB input. Use a smaller request or source, or run locally.",
            ));
        }
        let operation = operation.to_owned();
        let tenant_permit = self
            .tenant_semaphore
            .as_ref()
            .map(|s| {
                s.clone().try_acquire_owned().map_err(|_| {
                    policy::error(
                        "tenant_busy",
                        "This tenant already occupies its two compute slots",
                    )
                })
            })
            .transpose()?;
        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| policy::error("busy", "Compute slots are occupied; retry later"))?;
        if self.hosted && !trusted {
            #[cfg(feature = "sandbox")]
            {
                let sandbox = self.sandbox.clone().ok_or_else(|| {
                    policy::error(
                        "sandbox_unavailable",
                        "Hosted user-source compute requires a built sandbox",
                    )
                })?;
                let deadline = deadline.expect("hosted execution has a deadline");
                return tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let _tenant_permit = tenant_permit;
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        return Err(policy::error(
                            "deadline",
                            "Hosted request ran out of time before starting isolated computation.",
                        ));
                    }
                    sandbox.execute_for(&source, &operation, request, remaining)
                })
                .await
                .map_err(|e| policy::error("sandbox", e.to_string()))?;
            }
            #[cfg(not(feature = "sandbox"))]
            {
                return Err(policy::error(
                    "sandbox_unavailable",
                    "Hosted user-source compute requires --features sandbox and an embedded WASM artifact",
                ));
            }
        }
        let cache = self.compiled.clone();
        let flights = self.compilation_flights.clone();
        let library_version = library_version.to_owned();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _tenant_permit = tenant_permit;
            if deadline.is_some_and(|limit| tokio::time::Instant::now() >= limit) {
                return Err(policy::error(
                    "deadline",
                    "Hosted request ran out of time before starting native computation.",
                ));
            }
            (if trusted {
                serde_json::from_str(&source)
                    .map_err(|e| json!({"code":"invalid_library","message":e.to_string()}))
                    .and_then(|d| {
                        if matches!(
                            operation.strip_prefix("plan_").unwrap_or(&operation),
                            "fit_individual" | "fit_population"
                        ) {
                            return compute::execute_document(&d, &operation, request);
                        }
                        let key = format!("{library_version}:{}", hash(source.as_bytes()));
                        let compiled = crate::cache::compiled(&cache, &flights, key, || {
                            qspquest_modeling::document::CompiledDocument::compile(&d)
                                .map_err(compute::core_error)
                        })?;
                        compute::execute_compiled(&compiled, &d, &operation, request)
                    })
            } else {
                compute::execute(&source, &operation, request)
            })
            .map_err(|e| json!({"error":e}))
        });
        // The owned permit remains with blocking compute after timeout/cancellation.
        if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline,task).await.map_err(|_|policy::error("deadline","Hosted 45-second deadline exceeded. Native computation retains its slot until it finishes; run locally or use a managed job."))?.map_err(|e|policy::error("engine",e.to_string()))?
        } else {
            task.await
                .map_err(|e| policy::error("engine", e.to_string()))?
        }
    }
}
