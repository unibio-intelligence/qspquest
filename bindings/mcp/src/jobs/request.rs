//! Typed governed request admission; opaque platform inputs stay unchanged.
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Capability {
    Simulation,
    Fit,
    Scan,
}
impl Capability {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Simulation => "pkpd.simulate_scenario",
            Self::Fit => "pkpd.fit_data",
            Self::Scan => "pkpd.parameter_scan",
        }
    }
    fn parse(value: &Value) -> Result<Self, Value> {
        match value.as_str() {
            Some("pkpd.simulate_scenario") => Ok(Self::Simulation),
            Some("pkpd.fit_data") => Ok(Self::Fit),
            Some("pkpd.parameter_scan") => Ok(Self::Scan),
            _ => Err(failure("platform_request", "Supply an exact Run input for pkpd.simulate_scenario, pkpd.fit_data or pkpd.parameter_scan",
                "/platform_request/capability_id", "Use the exact capability_id for the governed operation. Inspect preview_job before submitting a priced Run.")),
        }
    }
}
fn failure(code: &str, message: &str, pointer: &str, suggestion: &str) -> Value {
    let mut value = crate::policy::error(code, message);
    value["error"]["expression"] = pointer.into();
    value["error"]["suggestion"] = suggestion.into();
    value
}
fn identifier(value: &str, pointer: &str) -> Result<(), Value> {
    super::safe_id(value).map_err(|mut error| {
        error["error"]["message"] = if pointer == "/idempotency_key" {
            "Invalid retry identifier"
        } else {
            "Invalid destination Project identifier"
        }
        .into();
        error["error"]["expression"] = pointer.into();
        error["error"]["suggestion"] =
            "Use 1 through 128 ASCII letters, digits, hyphens or underscores in this identifier."
                .into();
        error
    })
}

/// Local admission validates routing identity, never platform approval or science.
pub(super) struct RunInput {
    body: Value,
    capability: Capability,
    project: String,
}
impl RunInput {
    pub(super) fn parse(body: Value) -> Result<Self, Value> {
        if !body.is_object() {
            return Err(failure("platform_request", "Supply an exact Run input for pkpd.simulate_scenario, pkpd.fit_data or pkpd.parameter_scan",
                "/platform_request", "Supply platform_request as an object with capability_id and destination_project_id."));
        }
        let capability = Capability::parse(&body["capability_id"])?;
        let project = body["destination_project_id"].as_str().ok_or_else(||
            failure("platform_request", "A destination_project_id is required", "/platform_request/destination_project_id",
                "Choose the destination Project explicitly and supply its destination_project_id."))?.to_owned();
        identifier(&project, "/platform_request/destination_project_id")?;
        if body.to_string().len() > crate::policy::INPUT_LIMIT {
            return Err(failure("input_limit", "Platform input exceeds 1 MB", "/platform_request",
                "Reduce the submitted platform input to at most 1000000 serialized UTF-8 bytes; do not omit required study observations."));
        }
        Ok(Self {
            body,
            capability,
            project,
        })
    }
    pub(super) fn capability(&self) -> Capability {
        self.capability
    }
    pub(super) fn project(&self) -> &str {
        &self.project
    }
    pub(super) fn into_body(self) -> Value {
        self.body
    }
}

pub(super) struct SubmissionInput {
    key: String,
    request: RunInput,
}
impl SubmissionInput {
    pub(super) fn parse(input: Value) -> Result<Self, Value> {
        let key = input.get("idempotency_key").and_then(Value::as_str).ok_or_else(||
            failure("idempotency", "Supply a stable idempotency_key for retries; use a new key for a new priced Run",
                "/idempotency_key", "Reuse the same idempotency_key with the same reviewed Run input for retries; choose a new key for a new Run."))?.to_owned();
        identifier(&key, "/idempotency_key")?;
        let request = RunInput::parse(
            input
                .get("platform_request")
                .cloned()
                .unwrap_or(Value::Null),
        )?;
        let digest = request.body["expected_preview_digest"].as_str().ok_or_else(||
            failure("preview_required", "Run preview_job first and include the reviewed expected_preview_digest in platform_request",
                "/platform_request/expected_preview_digest", "Preview the exact Run input, review its price and approval requirements, then supply that preview's digest unchanged."))?;
        super::preview_digest(digest).map_err(|mut value| {
            value["error"]["expression"] = "/platform_request/expected_preview_digest".into();
            value
        })?;
        Ok(Self { key, request })
    }
    pub(super) fn into_parts(self) -> (RunInput, String) {
        (self.request, self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(capability: &str) -> Value {
        json!({"capability_id":capability,"destination_project_id":"project-1","inputs":[{"synthetic":true}],"parameters":{"method":"fixture"},"platform_extension":{"opaque":[1,2]}})
    }
    fn submission() -> Value {
        let mut body = run("pkpd.fit_data");
        body["expected_preview_digest"] = json!("a".repeat(64));
        json!({"platform_request":body,"idempotency_key":"stable-retry-key"})
    }

    #[test]
    fn all_three_supported_capabilities_preserve_complete_platform_inputs() {
        for (wire, capability) in [
            ("pkpd.simulate_scenario", Capability::Simulation),
            ("pkpd.fit_data", Capability::Fit),
            ("pkpd.parameter_scan", Capability::Scan),
        ] {
            let body = run(wire);
            let input = RunInput::parse(body.clone()).unwrap();
            assert!(input.capability() == capability);
            assert_eq!(input.capability().as_str(), wire);
            assert_eq!(input.project(), "project-1");
            assert_eq!(input.into_body().to_string(), body.to_string());
        }
    }

    #[test]
    fn invalid_shape_capability_and_project_are_located() {
        for (body, pointer) in [
            (json!(false), "/platform_request"),
            (
                json!({"capability_id":"pkpd.unknown"}),
                "/platform_request/capability_id",
            ),
            (
                json!({"capability_id":3}),
                "/platform_request/capability_id",
            ),
            (
                json!({"capability_id":"pkpd.fit_data"}),
                "/platform_request/destination_project_id",
            ),
            (
                json!({"capability_id":"pkpd.fit_data","destination_project_id":"../project"}),
                "/platform_request/destination_project_id",
            ),
        ] {
            let failure = match RunInput::parse(body) {
                Err(value) => value,
                Ok(_) => panic!("unexpected admission"),
            };
            assert_eq!(failure["error"]["expression"], pointer);
        }
    }

    #[test]
    fn retry_and_preview_identity_are_forwarded_without_replacement() {
        let submitted = submission();
        let (request, key) = SubmissionInput::parse(submitted.clone())
            .unwrap()
            .into_parts();
        assert_eq!(key, "stable-retry-key");
        let body = request.into_body();
        assert_eq!(body["expected_preview_digest"], "a".repeat(64));
        assert_eq!(body, submitted["platform_request"]);
        assert!(body.get("idempotency_key").is_none());
    }

    #[test]
    fn missing_or_unsafe_retry_keys_and_preview_digests_refuse() {
        for (field, value, pointer) in [
            ("idempotency_key", Value::Null, "/idempotency_key"),
            ("idempotency_key", json!("bad/key"), "/idempotency_key"),
            (
                "expected_preview_digest",
                Value::Null,
                "/platform_request/expected_preview_digest",
            ),
            (
                "expected_preview_digest",
                json!("A".repeat(64)),
                "/platform_request/expected_preview_digest",
            ),
            (
                "expected_preview_digest",
                json!("a".repeat(63)),
                "/platform_request/expected_preview_digest",
            ),
        ] {
            let mut input = submission();
            if field == "idempotency_key" {
                input[field] = value;
            } else {
                input["platform_request"][field] = value;
            }
            let failure = match SubmissionInput::parse(input) {
                Err(value) => value,
                Ok(_) => panic!("unexpected admission"),
            };
            assert_eq!(failure["error"]["expression"], pointer);
            assert!(failure["error"]["suggestion"].as_str().is_some());
        }
    }

    #[test]
    fn bounded_inputs_and_identifiers_refuse_without_serializing_values_in_errors() {
        let mut body = run("pkpd.fit_data");
        body["destination_project_id"] = json!("x".repeat(129));
        let failure = match RunInput::parse(body) {
            Err(value) => value,
            Ok(_) => panic!("unexpected admission"),
        };
        assert_eq!(failure["error"]["code"], "identifier");
        let mut body = run("pkpd.fit_data");
        body["private_field"] = json!("PRIVATE_FIXTURE_".repeat(100_000));
        let failure = match RunInput::parse(body) {
            Err(value) => value,
            Ok(_) => panic!("unexpected admission"),
        };
        assert_eq!(failure["error"]["code"], "input_limit");
        assert!(!failure.to_string().contains("PRIVATE_FIXTURE_"));
    }
}
