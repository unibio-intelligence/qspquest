//! Governed platform adapters. This layer never writes application data.
mod request;
use crate::policy::{error, Tier, INPUT_LIMIT};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{future::Future, pin::Pin};
pub type JobFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, Value>> + Send + 'a>>;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreditUnit {
    Credits,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreditPrice {
    #[schemars(range(max = 1_000_000_000))]
    pub credit_cost: u64,
    pub billing_unit: CreditUnit,
    pub pricing_model: String,
    pub pricing_note: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedRunState {
    Draft,
    AwaitingApproval,
    Ready,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
    Rejected,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedJobState {
    Queued,
    Leased,
    Running,
    RetryScheduled,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Expired,
}
impl ManagedJobState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Expired
        )
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ManagedRun {
    pub id: String,
    pub destination_project_id: String,
    pub capability_id: String,
    pub capability_version: String,
    pub state: ManagedRunState,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ManagedJob {
    pub id: String,
    pub run_id: String,
    pub destination_project_id: String,
    pub state: ManagedJobState,
    pub pricing: CreditPrice,
    /// Platform output records retain their own schema and access controls.
    pub staged_outputs: Vec<std::collections::BTreeMap<String, Value>>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ManagedPreview {
    pub capability_id: String,
    pub capability_version: String,
    pub valid: bool,
    pub preview_digest: String,
    pub approval_required: bool,
    pub pricing: CreditPrice,
    pub workbench_href: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ManagedSubmission {
    pub run: ManagedRun,
    pub jobs: Vec<ManagedJob>,
    pub pricing: CreditPrice,
    pub status_href: String,
    pub workbench_href: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ManagedArtifact {
    pub id: String,
    pub organization_id: String,
    pub project_id: String,
    pub run_id: Option<String>,
    pub job_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub checksum_hex: String,
    pub download_href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_retention: Option<crate::managed_artifacts::Retention>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct PreviewPayload {
    pub managed_preview: ManagedPreview,
    pub platform_response: std::collections::BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SubmissionPayload {
    pub managed_submission: ManagedSubmission,
    pub platform_response: std::collections::BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct JobPayload {
    pub managed_job: ManagedJob,
    pub platform_response: std::collections::BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ArtifactPayload {
    pub managed_artifact: ManagedArtifact,
    pub artifact_retrieval: crate::managed_artifacts::Retrieval,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_result_resource: Option<String>,
    pub platform_response: std::collections::BTreeMap<String, Value>,
}

fn decode<T: serde::de::DeserializeOwned>(response: &Value) -> Result<T, Value> {
    if !response.is_object() {
        return Err(error(
            "platform_response",
            "Platform response must be an object",
        ));
    }
    serde_json::from_value(response.get("data").unwrap_or(response).clone()).map_err(|_| {
        error(
            "platform_response",
            "Platform response has missing or invalid descriptor fields",
        )
    })
}
fn price_valid(price: &CreditPrice) -> bool {
    price.credit_cost <= 1_000_000_000
        && !price.pricing_model.is_empty()
        && !price.pricing_note.is_empty()
        && price.pricing_note.chars().count() <= 500
}
pub fn preview_digest(value: &str) -> Result<(), Value> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error("preview_required", "Preview the exact Run input, review its price, and supply its 64-character lowercase expected_preview_digest before submission"));
    }
    Ok(())
}
fn job_valid(job: &ManagedJob) -> bool {
    [&job.id, &job.run_id, &job.destination_project_id]
        .iter()
        .all(|id| safe_id(id).is_ok())
        && price_valid(&job.pricing)
}
fn invalid_descriptor() -> Value {
    error(
        "platform_response",
        "Platform returned an invalid managed descriptor",
    )
}

fn projection<T: Serialize>(field: &str, descriptor: &T, response: Value) -> Value {
    let mut value = json!({"platform_response":response});
    value[field] = serde_json::to_value(descriptor).expect("managed descriptor serialization");
    value
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Scope {
    pub user_id: String,
    pub tenant_id: String,
    pub tier: String,
    #[serde(skip)]
    pub bearer: String,
}
impl Scope {
    pub fn from_context(
        ctx: &rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Self, Value> {
        ctx.extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<Scope>())
            .cloned()
            .ok_or_else(|| error("session_required", "A verified managed session is required"))
    }
    pub fn admitted(&self) -> Result<Tier, Value> {
        let t = Tier::parse(&self.tier).map_err(|e| error("tier", e))?;
        if matches!(t, Tier::Free) {
            return Err(error("managed_unavailable","Free-tier requests use the public library and local fitting; no private platform call is made"));
        }
        Ok(t)
    }
}
pub trait JobBackend: Send + Sync {
    fn submit<'a>(&'a self, scope: &'a Scope, input: Value) -> JobFuture<'a>;
    fn get<'a>(&'a self, scope: &'a Scope, job_id: &'a str) -> JobFuture<'a>;
}
pub struct NoJobs;
impl JobBackend for NoJobs {
    fn submit<'a>(&'a self, _: &'a Scope, _: Value) -> JobFuture<'a> {
        Box::pin(async {
            Err(error(
                "jobs_unavailable",
                "Managed Job backend is not configured. Run the bounded request locally.",
            ))
        })
    }
    fn get<'a>(&'a self, _: &'a Scope, _: &'a str) -> JobFuture<'a> {
        Box::pin(async {
            Err(error(
                "jobs_unavailable",
                "Managed Job backend is not configured",
            ))
        })
    }
}
pub struct HttpJobBackend {
    pub(crate) base: String,
    pub(crate) client: reqwest::Client,
}
impl HttpJobBackend {
    pub fn configured() -> Result<Self, Value> {
        let base = std::env::var("QSPQUEST_PLATFORM_API_URL").map_err(|_| {
            error(
                "jobs_unavailable",
                "QSPQUEST_PLATFORM_API_URL is not configured",
            )
        })?;
        let base = crate::deployment::endpoint(&base).map_err(|e| error("configuration", e))?;
        Ok(Self {
            base: base.as_str().trim_end_matches('/').into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .map_err(|_| error("configuration", "Cannot build platform client"))?,
        })
    }
    pub async fn call(
        &self,
        scope: &Scope,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Value> {
        self.call_with_key(scope, method, path, body, None).await
    }
    async fn call_with_key(
        &self,
        scope: &Scope,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        key: Option<&str>,
    ) -> Result<Value, Value> {
        scope.admitted()?;
        if !path.starts_with('/') || path.contains("..") || path.contains('?') || path.contains('#')
        {
            return Err(error("path", "Unsafe platform route"));
        }
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&scope.bearer);
        if let Some(key) = key {
            req = req.header("Idempotency-Key", key)
        }
        if let Some(v) = body {
            if v.to_string().len() > INPUT_LIMIT {
                return Err(error("input_limit", "Managed payload exceeds 1 MB"));
            }
            req = req.json(&v)
        }
        let mut response = req
            .send()
            .await
            .map_err(|_| error("platform_unavailable", "Platform request failed"))?;
        let status = response.status();
        // Authorization failures must not reflect another tenant's upstream data.
        if !status.is_success() {
            return Err(
                json!({"error":{"code":"platform_rejected","status":status.as_u16(),"message":"The platform rejected the request; verify session, Project access and operation readiness"}}),
            );
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error("platform_unavailable", "Platform response interrupted"))?
        {
            if bytes.len() + chunk.len() > crate::policy::RESPONSE_LIMIT {
                return Err(error(
                    "response_limit",
                    "Platform response exceeds 256 KB; use a governed Artifact descriptor",
                ));
            }
            bytes.extend_from_slice(&chunk)
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| error("platform_response", "Platform returned a non-JSON response"))?;
        Ok(value)
    }
    pub(crate) async fn artifact_metadata(
        &self,
        scope: &Scope,
        id: &str,
    ) -> Result<(ManagedArtifact, Value), Value> {
        safe_id(id)?;
        let response = self
            .call(
                scope,
                reqwest::Method::GET,
                &format!("/artifacts/{id}"),
                None,
            )
            .await
            .map_err(|failure| {
                if failure["error"]["status"] == 410 {
                    error(
                        "managed_artifact_expired",
                        "The platform reports that this artifact retention has expired",
                    )
                } else {
                    failure
                }
            })?;
        let artifact: ManagedArtifact = decode(&response)?;
        if artifact.id != id
            || artifact.organization_id != scope.tenant_id
            || [
                &artifact.id,
                &artifact.organization_id,
                &artifact.project_id,
            ]
            .iter()
            .any(|id| safe_id(id).is_err())
            || artifact
                .run_id
                .as_deref()
                .is_some_and(|id| safe_id(id).is_err())
            || artifact
                .job_id
                .as_deref()
                .is_some_and(|id| safe_id(id).is_err())
            || preview_digest(&artifact.checksum_hex).is_err()
            || artifact.name.is_empty()
            || artifact.name.chars().count() > 500
            || artifact.kind.is_empty()
            || artifact.kind.len() > 128
            || artifact.media_type.is_empty()
            || artifact.media_type.len() > 200
            || artifact.download_href.len() > 2048
        {
            return Err(invalid_descriptor());
        }
        Ok((artifact, response))
    }
    pub async fn artifact(&self, scope: &Scope, id: &str) -> Result<Value, Value> {
        let (artifact, response) = self.artifact_metadata(scope, id).await?;
        let retrieval =
            crate::managed_artifacts::retrieval(&artifact, crate::managed_artifacts::clock()?)?;
        if matches!(
            retrieval,
            crate::managed_artifacts::Retrieval::Available { .. }
        ) {
            crate::managed_artifacts::download_url(&self.base, &artifact)?;
        }
        let uri = match &retrieval {
            crate::managed_artifacts::Retrieval::Available { descriptor } => {
                Some(descriptor.uri.clone())
            }
            _ => None,
        };
        let mut result = projection("managed_artifact", &artifact, response);
        result["artifact_retrieval"] =
            serde_json::to_value(retrieval).map_err(|_| invalid_descriptor())?;
        if let Some(uri) = uri {
            result["full_result_resource"] = json!(uri);
        }
        if result.to_string().len() > crate::policy::RESPONSE_LIMIT / 4 {
            let digest = crate::library::hash(result["platform_response"].to_string().as_bytes());
            result["platform_response"] = json!({"omitted_for_response_budget":true,"sha256":digest,
                "notice":"Unused upstream metadata is omitted. The typed artifact, explicit retrieval state and immutable full-result URI are retained; this hash is not a scientific identity."});
        }
        Ok(result)
    }
    pub async fn preview(&self, scope: &Scope, input: Value) -> Result<Value, Value> {
        scope.admitted()?;
        let input = request::RunInput::parse(input)?;
        let capability = input.capability();
        let response = self
            .call(
                scope,
                reqwest::Method::POST,
                "/runs/preview",
                Some(input.into_body()),
            )
            .await?;
        let preview: ManagedPreview = decode(&response)?;
        if preview.capability_id != capability.as_str()
            || preview_digest(&preview.preview_digest).is_err()
            || !price_valid(&preview.pricing)
        {
            return Err(invalid_descriptor());
        }
        Ok(projection("managed_preview", &preview, response))
    }
    pub(crate) async fn get_typed(&self, scope: &Scope, id: &str) -> Result<JobProjection, Value> {
        safe_id(id)?;
        let response = self
            .call(scope, reqwest::Method::GET, &format!("/jobs/{id}"), None)
            .await?;
        let job: ManagedJob = decode(&response)?;
        if job.id != id || !job_valid(&job) {
            return Err(invalid_descriptor());
        }
        Ok(JobProjection { job, response })
    }
}

/// A validated Job descriptor paired with its unchanged platform response.
pub(crate) struct JobProjection {
    job: ManagedJob,
    response: Value,
}
impl JobProjection {
    pub(crate) fn terminal(&self) -> bool {
        self.job.state.terminal()
    }
    pub(crate) fn into_value(self) -> Value {
        projection("managed_job", &self.job, self.response)
    }
}

pub fn safe_id(id: &str) -> Result<(), Value> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(error("identifier", "Invalid Job/Artifact identifier"));
    }
    Ok(())
}
impl JobBackend for HttpJobBackend {
    fn submit<'a>(&'a self, scope: &'a Scope, input: Value) -> JobFuture<'a> {
        Box::pin(async move {
            scope.admitted()?;
            // The platform atomically checks the caller's unchanged preview
            // digest, approval policy and stable retry key.
            let (request, key) = request::SubmissionInput::parse(input)?.into_parts();
            let capability = request.capability();
            let project = request.project().to_owned();
            let response = self
                .call_with_key(
                    scope,
                    reqwest::Method::POST,
                    "/runs",
                    Some(request.into_body()),
                    Some(&key),
                )
                .await?;
            let submission: ManagedSubmission = decode(&response)?;
            if safe_id(&submission.run.id).is_err()
                || safe_id(&submission.run.destination_project_id).is_err()
                || submission.run.destination_project_id != project
                || submission.run.capability_id != capability.as_str()
                || !price_valid(&submission.pricing)
                || !submission.jobs.iter().all(job_valid)
                || submission.jobs.iter().any(|job| {
                    job.run_id != submission.run.id
                        || job.destination_project_id != submission.run.destination_project_id
                })
            {
                return Err(invalid_descriptor());
            }
            Ok(projection("managed_submission", &submission, response))
        })
    }
    fn get<'a>(&'a self, scope: &'a Scope, id: &'a str) -> JobFuture<'a> {
        Box::pin(async move {
            self.get_typed(scope, id)
                .await
                .map(JobProjection::into_value)
        })
    }
}
pub struct PlatformModelSource {
    backend: HttpJobBackend,
}
impl PlatformModelSource {
    pub fn configured() -> Result<Self, Value> {
        Ok(Self {
            backend: HttpJobBackend::configured()?,
        })
    }
    pub async fn search(&self, scope: &Scope) -> Result<Value, Value> {
        let response = self
            .backend
            .call(scope, reqwest::Method::GET, "/pkpd/models", None)
            .await?;
        let data = response.get("data").unwrap_or(&response);
        if !data["models"]
            .as_array()
            .is_some_and(|models| models.iter().all(Value::is_object))
        {
            return Err(invalid_descriptor());
        }
        Ok(response)
    }
    pub async fn search_discovery(
        &self,
        scope: &Scope,
        query: &str,
        filters: &Value,
    ) -> Result<Vec<crate::discovery::PlatformDiscoveryModel>, Value> {
        let response = self.search(scope).await?;
        crate::discovery::platform_models(response.get("data").unwrap_or(&response), query, filters)
            .map_err(|message| crate::policy::error("platform_response", message))
    }
    pub async fn model(&self, scope: &Scope, id: &str) -> Result<Value, Value> {
        safe_id(id)?;
        let response = self
            .backend
            .call(
                scope,
                reqwest::Method::GET,
                &format!("/pkpd/models/{id}"),
                None,
            )
            .await?;
        if !response.is_object() || !response.get("data").unwrap_or(&response).is_object() {
            return Err(invalid_descriptor());
        }
        Ok(response)
    }
}
/// A fixed, validated resolver; client identity comes only from its response.
pub struct SessionResolver {
    url: reqwest::Url,
    client: reqwest::Client,
}
impl SessionResolver {
    pub fn configured() -> Result<Self, String> {
        let value = std::env::var("QSPQUEST_PLATFORM_SESSION_URL")
            .map_err(|_| "QSPQUEST_PLATFORM_SESSION_URL is required for managed HTTP")?;
        Ok(Self {
            url: crate::deployment::endpoint(&value)?,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .map_err(|_| "Cannot build session resolver client")?,
        })
    }
    /// Reachability only: this sends no credentials and validates no session.
    pub async fn probe(&self) -> Result<(), String> {
        let response = self
            .client
            .get(self.url.clone())
            .send()
            .await
            .map_err(|_| "Managed session resolver is unavailable; check its endpoint and TLS")?;
        match response.status().as_u16() {
            200 | 401 | 403 => Ok(()),
            _ => {
                Err("Managed session resolver readiness failed; check the configured route".into())
            }
        }
    }
}
pub async fn authenticate(
    axum::extract::State(resolver): axum::extract::State<std::sync::Arc<SessionResolver>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if request.uri().path() == "/healthz" || request.uri().path().starts_with("/assets/") {
        return next.run(request).await;
    }
    let Some(bearer) = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned)
    else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(error("session_required", "Bearer session required")),
        )
            .into_response();
    };
    let result = async {
        let response = resolver
            .client
            .get(resolver.url.clone())
            .bearer_auth(&bearer)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len() + chunk.len() > 16_000 {
                return Err(());
            }
            bytes.extend_from_slice(&chunk)
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let data = value.get("data").unwrap_or(&value);
        let mut scope: Scope = if data.get("principal").is_some() {
            let id = data["principal"]["id"].as_str().ok_or(())?.to_owned();
            Scope {
                user_id: id.clone(),
                tenant_id: data["principal"]["organization_id"]
                    .as_str()
                    .unwrap_or(&id)
                    .to_owned(),
                tier: data["usage_plan"].as_str().ok_or(())?.to_owned(),
                bearer: String::new(),
            }
        } else {
            serde_json::from_value(data.clone()).map_err(|_| ())?
        };
        safe_id(&scope.user_id).map_err(|_| ())?;
        safe_id(&scope.tenant_id).map_err(|_| ())?;
        Tier::parse(&scope.tier).map_err(|_| ())?;
        scope.bearer = bearer;
        Ok(scope)
    }
    .await;
    match result {
        Ok(scope) => {
            request.extensions_mut().insert(scope);
            next.run(request).await
        }
        Err(()) => (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(error(
                "session_required",
                "Session could not be verified by the configured platform resolver",
            )),
        )
            .into_response(),
    }
}

// Weak entries avoid retaining tenant identity after all in-flight requests finish.
// Active work holds its owned slot even if the caller disconnects.
pub fn tenant_slots(id: &str) -> std::sync::Arc<tokio::sync::Semaphore> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static SLOTS: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Semaphore>>>> = OnceLock::new();
    let mut slots = SLOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    slots.retain(|_, s| s.strong_count() > 0);
    if let Some(s) = slots.get(id).and_then(Weak::upgrade) {
        return s;
    }
    let s = Arc::new(tokio::sync::Semaphore::new(2));
    slots.insert(id.to_owned(), Arc::downgrade(&s));
    s
}

#[cfg(test)]
mod typed_projection_tests {
    use super::*;

    #[test]
    fn typed_job_states_and_original_platform_payload_are_preserved() {
        for (state, terminal) in [
            ("queued", false),
            ("leased", false),
            ("running", false),
            ("retry_scheduled", false),
            ("cancel_requested", false),
            ("succeeded", true),
            ("failed", true),
            ("cancelled", true),
            ("timed_out", true),
            ("expired", true),
        ] {
            let response = json!({"data":{"id":"job-1","run_id":"run-1",
                "destination_project_id":"project-1","state":state,
                "pricing":{"credit_cost":42,"billing_unit":"credits","pricing_model":"synthetic","pricing_note":"Synthetic fixed operation"},
                "staged_outputs":[],"extra_metadata":{"nested":[1,2]}}});
            let job: ManagedJob = decode(&response).unwrap();
            assert!(job_valid(&job));
            let value = JobProjection {
                job,
                response: response.clone(),
            };
            assert_eq!(value.terminal(), terminal);
            let value = value.into_value();
            assert_eq!(value["managed_job"]["state"], state);
            assert_eq!(value["platform_response"].to_string(), response.to_string());
            let _: JobPayload = serde_json::from_value(value).unwrap();
        }
    }
}
