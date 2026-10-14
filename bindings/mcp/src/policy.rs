use serde_json::{json, Value};
#[derive(Clone, Copy, Debug)]
pub enum Tier {
    Free,
    Pro,
    Enterprise,
    Admin,
}
impl Tier {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "free" => Ok(Self::Free),
            "pro" => Ok(Self::Pro),
            "enterprise" => Ok(Self::Enterprise),
            "admin" => Ok(Self::Admin),
            _ => Err("tier must be free, pro, enterprise or admin".into()),
        }
    }
    pub fn scan_runs(self) -> usize {
        match self {
            Self::Free => 25,
            Self::Pro => 100,
            Self::Enterprise => 225,
            Self::Admin => 400,
        }
    }
}
pub const INPUT_LIMIT: usize = 1_000_000;
pub const RESPONSE_LIMIT: usize = 256_000;
/// Shared ceilings for record mapping and native population request admission.
pub fn population_limits(hosted: bool) -> (usize, usize) {
    crate::execution_policy::Profile::from_hosted(hosted).population_limits()
}

pub fn error(code: &str, message: impl Into<String>) -> Value {
    json!({"error":{"code":code,"message":message.into(),"suggestion":"Read qspquest://guide/requests; use explicit units, reduce the request, run locally, or submit a managed job."}})
}
/// Validate inline workload limits before inserting any run defaults.
/// Rejected requests retain their submitted bytes and report a JSON pointer.
pub fn admit(v: &mut Value, hosted: bool) -> Result<(), Value> {
    crate::execution_policy::admit(v, crate::execution_policy::Profile::from_hosted(hosted))
        .map_err(crate::execution_policy::Failure::into_value)
}

/// Hosted interactive work is bounded separately from the isolated/local engine.
/// Large catalog models remain supported by local execution and governed jobs.
pub fn async_route(model: &Value, operation: &str) -> Option<Value> {
    let states = model["states"].as_array().map_or(0, Vec::len);
    if states < 64 {
        return None;
    }
    let (tool, capability) = match operation {
        "simulate" => ("submit_simulation", "pkpd.simulate_scenario"),
        "scan_parameters" => ("submit_scan", "pkpd.parameter_scan"),
        "fit_population" | "fit_individual" | "fit_scalar" => {
            ("submit_population_fit", "pkpd.fit_data")
        }
        _ => return None,
    };
    Some(
        json!({"error":{"code":"async_required","message":format!("This {states}-state model exceeds hosted interactive admission. Run it through local stdio or an authorized managed endpoint."),"suggestion":"For offline work, start qspquest-mcp --profile local --library-dir PATH --jobs-dir PRIVATE_JOB_PATH --artifact-dir PRIVATE_RESULT_PATH using distinct private directories, then use submit_local_job and get_local_job. Managed submission separately requires platform qualification, a Project, reviewed pricing and a stable retry key."},
        "execution_route":{"kind":"governed_job","tool":tool,"capability_id":capability,"states":states,"requires_verified_session":true,"requires_platform_qualification":true,"local_execution_supported":true},
        "latency_contract":{"interactive_execution_target_ms":500,"job_admission_target_ms":500,"job_execution_target_ms":null}}),
    )
}
#[cfg(test)]
mod latency_policy_tests {
    use super::*;
    #[test]
    fn heavy_work_is_explicitly_routed_without_reclassifying_it_as_a_result() {
        assert!(async_route(&json!({"states":vec![0;63]}), "simulate").is_none());
        let v = async_route(&json!({"states":vec![0;725]}), "simulate").unwrap();
        assert_eq!(v["error"]["code"], "async_required");
        assert_eq!(v["execution_route"]["tool"], "submit_simulation");
        assert!(v.get("outputs").is_none());
    }
}
