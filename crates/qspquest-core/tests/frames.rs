use qspquest_core::frames;
use serde_json::json;

fn result() -> serde_json::Value {
    json!({"schema":"qspquest.result/v0.1","time_unit":"h","identity":{"model_content_hash":"model-hash"},"times":[0.,1.,1.,2.],"sides":["post","pre","post","post"],"outputs":[{"name":"TIME","unit":"mg/L","values":[0.,1.,3.,null]}]})
}
#[test]
fn trajectory_preserves_units_names_missing_values_and_jump_order() {
    let f = frames::trajectory(&result()).unwrap();
    assert_eq!(
        f.columns,
        ["time", "side", "output", "value", "unit", "series"]
    );
    assert_eq!(f.rows.len(), 4);
    assert_eq!(f.rows[1][1], "pre");
    assert_eq!(f.rows[2][1], "post");
    assert_eq!(f.rows[3][3], serde_json::Value::Null);
    assert_eq!(f.metadata["identity"]["model_content_hash"], "model-hash");
    assert_eq!(f.records()[0]["output"], "TIME");
    let mut invalid = result();
    invalid["sides"][1] = json!("post");
    assert!(frames::trajectory(&invalid).is_err());
    invalid = result();
    invalid["outputs"][0]["values"] = json!([1.]);
    assert!(frames::trajectory(&invalid).is_err());
}
#[test]
fn events_share_core_expansion_units_repeats_and_rate_before_bioavailability() {
    let request = json!({"regimen":{"end":{"value":10.,"unit":"h"},"samples":[],"rtol":1e-6,"atol":1e-8,
        "administrations":[{"target":"central","time":{"value":60.,"unit":"min"},"amount":{"value":100.,"unit":"mg"},"delivery":{"kind":"infusion","span":{"kind":"rate","rate":{"value":50.,"unit":"mg/h"}}},"repeat":{"interval":{"value":3.,"unit":"h"},"additional":1},"lag":{"value":30.,"unit":"min"},"bioavailability":0.5}]}});
    let f = frames::events(&request, Some("min")).unwrap();
    let records = f.records();
    assert_eq!(records.len(), 6);
    assert_eq!(records[0]["kind"], "lag");
    assert_eq!(records[1]["time"], 90.);
    assert_eq!(records[2]["time"], 210.);
    assert_eq!(records[1]["duration"], 120.);
    assert_eq!(records[1]["amount"], 100.);
    assert_eq!(records[4]["time"], 270.);
    assert_eq!(f.metadata["time_unit"], "min");
}
#[test]
fn blq_and_failed_replicates_remain_explicit() {
    let f=frames::convert(&json!({"schema":"qspquest.bootstrap/v0.1","replicates":[{"replicate":0,"status":"failed","parameters":null},{"replicate":1,"status":"converged","parameters":{"cl":{"value":2.,"unit":"L/h"}}}]}),"bootstrap").unwrap();
    assert_eq!(f.rows.len(), 2);
    assert_eq!(f.records()[0]["status"], "failed");
    assert_eq!(f.records()[0]["value"], serde_json::Value::Null);
    let vpc = json!({"schema":"qspquest.vpc/v0.1","bins":[{"stratum":"all","start":0.,"end":1.,"observations":5,"quantiles":[{"percentile":0.5,"observed":null,"simulated":null,"confidence_interval":null}],"observed_blq_fraction":1.,"simulated_blq_fraction":1.,"blq_confidence_interval":[1.,1.]}]});
    let f = frames::convert(&vpc, "vpc_bins").unwrap();
    assert_eq!(f.records()[0]["observed"], serde_json::Value::Null);
    assert_eq!(f.records()[0]["obs_blq"], 1.);
}
