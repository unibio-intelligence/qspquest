//! Governed submission and polling from an authenticated MCP request context.
use super::*;

impl Server {
    pub(super) async fn submit_job(&self, p: JobInput, ctx: RequestContext<RoleServer>) -> Value {
        use crate::jobs::JobBackend;
        let r = async {
            let scope = crate::jobs::Scope::from_context(&ctx)?;
            scope.admitted()?;
            let backend = crate::jobs::HttpJobBackend::configured()?;
            backend.submit(&scope,json!({"platform_request":p.platform_request,"idempotency_key":p.idempotency_key})).await
        };
        r.await.unwrap_or_else(|e| e)
    }
    pub(super) async fn poll_job(
        &self,
        p: JobId,
        ctx: RequestContext<RoleServer>,
        wait: bool,
    ) -> Value {
        let r = async {
            let scope = crate::jobs::Scope::from_context(&ctx)?;
            scope.admitted()?;
            let backend = crate::jobs::HttpJobBackend::configured()?;
            let end = std::time::Instant::now()
                + std::time::Duration::from_secs(if wait {
                    p.seconds.unwrap_or(20).min(40)
                } else {
                    0
                });
            loop {
                let value = backend.get_typed(&scope, &p.job_id).await?;
                if !wait || std::time::Instant::now() >= end || value.terminal() {
                    return Ok(value.into_value());
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await
            }
        };
        r.await.unwrap_or_else(|e| e)
    }
}
