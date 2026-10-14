//! Native and isolated execution dispatch, permits, deadlines and compilation reuse.
use super::*;

impl Server {
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
        policy::admit(&mut request, self.hosted)?;
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
                return tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let _tenant_permit = tenant_permit;
                    sandbox.execute(&source, &operation, request)
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
        if self.hosted {
            tokio::time::timeout(std::time::Duration::from_secs(45),task).await.map_err(|_|policy::error("deadline","Hosted 45-second deadline exceeded. Native computation retains its slot until it finishes; run locally or use a managed job."))?.map_err(|e|policy::error("engine",e.to_string()))?
        } else {
            task.await
                .map_err(|e| policy::error("engine", e.to_string()))?
        }
    }
}
