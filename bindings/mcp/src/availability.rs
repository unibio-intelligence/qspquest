//! Model availability is distinct from qualification of the active engine build.
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "qspquest.model-availability/v1";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Availability {
    model_id: String,
    archive_version: String,
    selected_version: String,
    /// Explicit source identity only for the bounded legacy scalar fixture.
    #[serde(default)]
    source_model_version: Option<String>,
    archive_sha256: String,
    availability: String,
    publication_review: String,
    preset_ids: Vec<String>,
    unit_status: String,
    fitting_supported: bool,
    fit_ready: bool,
    evidence_id: String,
    current_engine_qualification: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    schema: String,
    #[serde(deserialize_with = "unique_records")]
    records: BTreeMap<String, Availability>,
}

fn unique_records<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Availability>, D::Error> {
    struct Records;
    impl<'de> serde::de::Visitor<'de> for Records {
        type Value = BTreeMap<String, Availability>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("unique model/version availability records")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut records = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, Availability>()? {
                if records.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "Duplicate availability record key",
                    ));
                }
            }
            Ok(records)
        }
    }
    deserializer.deserialize_map(Records)
}

fn sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl Availability {
    fn validate(&self) -> Result<(), String> {
        if [
            &self.model_id,
            &self.archive_version,
            &self.selected_version,
        ]
        .iter()
        .any(|s| s.is_empty())
            || self
                .source_model_version
                .as_ref()
                .is_some_and(|version| version.is_empty() || version == &self.archive_version)
            || !sha256(&self.archive_sha256)
            || !self.evidence_id.strip_prefix("sha256:").is_some_and(sha256)
            || !matches!(
                self.availability.as_str(),
                "dynamic" | "source_replay" | "inspection_only"
            )
            || !matches!(
                self.publication_review.as_str(),
                "pending" | "unrecorded" | "approved"
            )
            || !matches!(
                self.unit_status.as_str(),
                "physically_qualified" | "source_scale_unqualified" | "unqualified"
            )
            || self.current_engine_qualification != "not_assessed"
            || self.preset_ids.iter().any(String::is_empty)
            || self.preset_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || (self.availability == "inspection_only") != self.preset_ids.is_empty()
            || self.fit_ready && !self.fitting_supported
            || self.fitting_supported && self.availability != "dynamic"
        {
            return Err(format!(
                "Contradictory or unsupported availability: {}/{}",
                self.model_id, self.archive_version
            ));
        }
        Ok(())
    }

    pub fn describe(&self) -> Value {
        let mut operations = vec!["inspect"];
        if self.availability != "inspection_only" {
            operations.push("run_preset");
        }
        if self.availability == "dynamic" {
            operations.push("exploratory_simulation");
        }
        if self.fitting_supported {
            operations.push("fit");
        }
        let mut blockers = vec!["current_engine_qualification_not_assessed".to_owned()];
        if self.publication_review != "approved" {
            blockers.push(format!("publication_review_{}", self.publication_review));
        }
        if self.unit_status != "physically_qualified" {
            blockers.push("physical_units_not_qualified".into());
        }
        if !self.fitting_supported {
            blockers.push("fitting_not_supported".into());
        }
        if self.availability == "inspection_only" {
            blockers.push("simulation_unavailable".into());
        }
        if self.availability == "source_replay" {
            blockers.push("dynamic_simulation_not_qualified".into());
        }
        let mut description = json!({"schema":SCHEMA, "model_id":self.model_id, "archive_version":self.archive_version,
            "selected_version":self.selected_version,"archive_sha256":self.archive_sha256,
            "availability":self.availability,"publication_review":self.publication_review,
            "preset_ids":self.preset_ids,"unit_status":self.unit_status,
            "fitting_supported":self.fitting_supported,"fit_ready":self.fit_ready,
            "evidence_id":self.evidence_id,"current_engine_qualification":self.current_engine_qualification,
            "selected":self.archive_version == self.selected_version,
            "supported_operations":operations,"blockers":blockers});
        if let Some(version) = &self.source_model_version {
            description["source_model_version"] = json!(version);
        }
        description
    }

    pub fn verify_archive(
        &self,
        index: &Value,
        digest: &str,
        fitting: &Value,
    ) -> Result<(), String> {
        let native = &index["native_execution"];
        let mut presets = native["scenarios"]
            .as_object()
            .ok_or("Availability requires native scenario inventory")?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        presets.sort();
        let unit = if native["dimensional_qualification"] == true {
            "physically_qualified"
        } else if native["coordinates"] == "unchanged_source_numeric_values" {
            "source_scale_unqualified"
        } else {
            "unqualified"
        };
        let legacy = fitting["schema"] == "qspquest.mcp-fitting-contract/v0.1";
        let identity_matches = match &self.source_model_version {
            None => index["model_version"] == self.archive_version,
            Some(version) => {
                legacy
                    && index["version"] == self.archive_version
                    && index["model_version"] == *version
            }
        };
        if index["model_id"] != self.model_id
            || !identity_matches
            || digest != self.archive_sha256
            || presets != self.preset_ids
            || (native["model"].as_str().is_none()) != (self.availability == "inspection_only")
            || unit != self.unit_status
        {
            return Err(
                "Availability identity, execution, presets or units differ from archive".into(),
            );
        }
        for (key, expected) in [
            ("fitting_supported", self.fitting_supported),
            ("fit_ready", self.fit_ready),
        ] {
            // Only the existing legacy scalar profile stores optional flags in
            // its hash-indexed archive index. Modern contracts remain explicit.
            let flag = fitting
                .get(key)
                .or_else(|| legacy.then(|| index.get(key)).flatten());
            if flag.map_or(Some(false), Value::as_bool) != Some(expected) {
                return Err("Availability fitting flags differ from archive contract".into());
            }
        }
        Ok(())
    }

    pub fn evidence_matches(&self, bytes: &[u8]) -> bool {
        self.evidence_id == format!("sha256:{}", crate::library::hash(bytes))
    }
}

impl Registry {
    pub fn parse(bytes: &[u8], catalog: &Value) -> Result<Self, String> {
        if bytes.len() > 1_000_000 {
            return Err("Availability registry exceeds 1 MB".into());
        }
        let registry: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if registry.schema != SCHEMA {
            return Err("Unsupported availability registry schema".into());
        }
        let rows = catalog["models"]
            .as_array()
            .ok_or("Catalog models missing")?;
        if registry.records.len() > 1024
            || registry
                .records
                .values()
                .filter(|r| r.archive_version == r.selected_version)
                .count()
                != rows.len()
        {
            return Err("Availability selected identity inventory differs from catalog".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for row in rows {
            if !seen.insert(row["id"].as_str().ok_or("Model ID missing")?) {
                return Err("Duplicate catalog identity".into());
            }
            let record = registry.for_model(row)?;
            let mut presets = row["scenario_ids"]
                .as_array()
                .ok_or("Catalog scenarios missing")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or("Scenario ID must be text")
                        .map(str::to_owned)
                })
                .collect::<Result<Vec<_>, _>>()?;
            presets.sort();
            if record.archive_version != record.selected_version
                || row
                    .get("source_model_version")
                    .is_some_and(|value| !value.is_null() && !value.is_string())
                || row.get("source_model_version").and_then(Value::as_str)
                    != record.source_model_version.as_deref()
                || row["omex_sha256"] != record.archive_sha256
                || presets != record.preset_ids
                || row["publication_status"] != record.publication_review
                || row["fitting_supported"] != record.fitting_supported
                || row["fit_ready"] != record.fit_ready
            {
                return Err(
                    "Catalog status, selected version, presets or digest differs from availability"
                        .into(),
                );
            }
        }
        for (key, record) in &registry.records {
            record.validate()?;
            if key != &format!("{}/{}", record.model_id, record.archive_version)
                || !rows.iter().any(|row| {
                    row["id"] == record.model_id && row["version"] == record.selected_version
                })
            {
                return Err("Availability key or selected version differs from catalog".into());
            }
        }
        Ok(registry)
    }

    pub fn for_model(&self, row: &Value) -> Result<&Availability, String> {
        let key = format!(
            "{}/{}",
            row["id"].as_str().ok_or("Model ID missing")?,
            row["version"].as_str().ok_or("Model version missing")?
        );
        self.records
            .get(&key)
            .ok_or_else(|| format!("Availability record missing: {key}"))
    }

    pub fn counts(&self) -> Value {
        let current = self
            .records
            .values()
            .filter(|r| r.archive_version == r.selected_version)
            .collect::<Vec<_>>();
        let mut classes = BTreeMap::<String, usize>::new();
        for record in &current {
            *classes.entry(record.availability.clone()).or_default() += 1;
        }
        json!({"selected_model_identities":current.len(),"versioned_archives":self.records.len(),
            "selected_preset_requests":current.iter().map(|r| r.preset_ids.len()).sum::<usize>(),
            "all_archive_preset_requests":self.records.values().map(|r| r.preset_ids.len()).sum::<usize>(),
            "selected_availability":classes,"selected_fitting_supported":current.iter().filter(|r| r.fitting_supported).count(),
            "selected_fit_ready":current.iter().filter(|r| r.fit_ready).count()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, Value) {
        let record = json!({"model_id":"synthetic","archive_version":"1","selected_version":"1",
            "archive_sha256":"a".repeat(64),"availability":"dynamic","publication_review":"pending",
            "preset_ids":["starter"],"unit_status":"source_scale_unqualified",
            "fitting_supported":false,"fit_ready":false,"evidence_id":format!("sha256:{}", "b".repeat(64)),
            "current_engine_qualification":"not_assessed"});
        let registry = json!({"schema":SCHEMA,"records":{"synthetic/1":record}});
        let catalog = json!({"models":[{"id":"synthetic","version":"1","omex_sha256":"a".repeat(64),
            "scenario_ids":["starter"],"publication_status":"pending","fitting_supported":false,"fit_ready":false}]});
        (registry, catalog)
    }

    #[test]
    fn discovery_preserves_distinct_execution_units_review_and_qualification() {
        let (registry, catalog) = fixture();
        let parsed = Registry::parse(&serde_json::to_vec(&registry).unwrap(), &catalog).unwrap();
        let record = parsed.for_model(&catalog["models"][0]).unwrap().describe();
        assert_eq!(record["publication_review"], "pending");
        assert_eq!(record["current_engine_qualification"], "not_assessed");
        assert_eq!(
            record["supported_operations"],
            json!(["inspect", "run_preset", "exploratory_simulation"])
        );
        assert_eq!(parsed.counts()["selected_model_identities"], 1);
        assert_eq!(parsed.counts()["versioned_archives"], 1);
        assert_eq!(parsed.counts()["selected_preset_requests"], 1);
        assert_eq!(parsed.counts()["selected_fit_ready"], 0);
    }

    #[test]
    fn contradictory_and_future_records_do_not_silently_gain_capabilities() {
        let (registry, catalog) = fixture();
        for (field, value) in [
            ("availability", json!("inspection_only")),
            ("fit_ready", json!(true)),
            ("fitting_supported", json!("false")),
            ("current_engine_qualification", json!("passed")),
            ("selected_version", json!("2")),
            ("preset_ids", json!(["starter", "starter"])),
            ("unrecognized_field", json!(true)),
        ] {
            let mut changed = registry.clone();
            changed["records"]["synthetic/1"][field] = value;
            assert!(
                Registry::parse(&serde_json::to_vec(&changed).unwrap(), &catalog).is_err(),
                "{field}"
            );
        }
        let mut changed = catalog.clone();
        changed["models"][0]["publication_status"] = json!("approved");
        assert!(Registry::parse(&serde_json::to_vec(&registry).unwrap(), &changed).is_err());
    }

    #[test]
    fn duplicate_registry_keys_are_rejected_instead_of_replacing_records() {
        let (registry, catalog) = fixture();
        let record = registry["records"]["synthetic/1"].to_string();
        let raw = format!(
            r#"{{"schema":"{SCHEMA}","records":{{"synthetic/1":{record},"synthetic/1":{record}}}}}"#
        );
        assert!(Registry::parse(raw.as_bytes(), &catalog).is_err());
    }

    #[test]
    fn changed_archive_units_identity_or_fitting_invalidates_availability() {
        let (registry, catalog) = fixture();
        let parsed = Registry::parse(&serde_json::to_vec(&registry).unwrap(), &catalog).unwrap();
        let record = parsed.for_model(&catalog["models"][0]).unwrap();
        let index = json!({"model_id":"synthetic","model_version":"1", "native_execution":{
            "model":"native/model.json","scenarios":{"starter":"native/scenarios/starter.json"},
            "coordinates":"unchanged_source_numeric_values","dimensional_qualification":false}});
        record
            .verify_archive(&index, &"a".repeat(64), &json!({}))
            .unwrap();
        for (field, value) in [
            ("dimensional_qualification", json!(true)),
            ("scenarios", json!({})),
            ("model", Value::Null),
        ] {
            let mut changed = index.clone();
            changed["native_execution"][field] = value;
            assert!(
                record
                    .verify_archive(&changed, &"a".repeat(64), &json!({}))
                    .is_err(),
                "{field}"
            );
        }
        assert!(record
            .verify_archive(&index, &"c".repeat(64), &json!({}))
            .is_err());
        assert!(record
            .verify_archive(&index, &"a".repeat(64), &json!({"fit_ready":true}))
            .is_err());
        assert!(!record.evidence_matches(b"unrelated historical evidence"));
    }
    #[test]
    fn explicit_legacy_source_identity_does_not_relax_modern_archive_identity() {
        let (mut registry, mut catalog) = fixture();
        registry["records"]["synthetic/1"]["source_model_version"] = json!("0");
        catalog["models"][0]["source_model_version"] = json!("0");
        let parsed = Registry::parse(&serde_json::to_vec(&registry).unwrap(), &catalog).unwrap();
        let record = parsed.for_model(&catalog["models"][0]).unwrap();
        let index = json!({"model_id":"synthetic","model_version":"0","version":"1","native_execution":{
            "model":"native/model.json","scenarios":{"starter":"native/scenarios/starter.json"},
            "coordinates":"unchanged_source_numeric_values","dimensional_qualification":false}});
        let legacy = json!({"schema":"qspquest.mcp-fitting-contract/v0.1"});
        record
            .verify_archive(&index, &"a".repeat(64), &legacy)
            .unwrap();
        assert_eq!(record.describe()["source_model_version"], "0");
        for fitting in [json!({}), json!({"schema":"qspquest.library-fitting/v1"})] {
            assert!(record
                .verify_archive(&index, &"a".repeat(64), &fitting)
                .is_err());
        }
        for key in ["version", "model_version"] {
            let mut changed = index.clone();
            changed[key] = json!("wrong");
            assert!(record
                .verify_archive(&changed, &"a".repeat(64), &legacy)
                .is_err());
        }
        let mut changed = catalog;
        changed["models"][0]["source_model_version"] = json!("wrong");
        assert!(Registry::parse(&serde_json::to_vec(&registry).unwrap(), &changed).is_err());
    }
    #[test]
    fn legacy_index_flags_are_bounded_and_explicit_contract_flags_take_precedence() {
        let (mut registry, mut catalog) = fixture();
        for key in ["fit_ready", "fitting_supported"] {
            registry["records"]["synthetic/1"][key] = json!(true);
            catalog["models"][0][key] = json!(true);
        }
        let parsed = Registry::parse(&serde_json::to_vec(&registry).unwrap(), &catalog).unwrap();
        let record = parsed.for_model(&catalog["models"][0]).unwrap();
        let index = json!({"model_id":"synthetic","model_version":"1","fit_ready":true,"fitting_supported":true,
            "native_execution":{"model":"native/model.json","scenarios":{"starter":"native/scenarios/starter.json"},
            "coordinates":"unchanged_source_numeric_values","dimensional_qualification":false}});
        let legacy = json!({"schema":"qspquest.mcp-fitting-contract/v0.1"});
        record
            .verify_archive(&index, &"a".repeat(64), &legacy)
            .unwrap();
        for fitting in [
            json!({"schema":"qspquest.library-fitting/v1"}),
            json!({"schema":"qspquest.mcp-fitting-contract/v0.1","fit_ready":false}),
            json!({"schema":"qspquest.mcp-fitting-contract/v0.1","fit_ready":null}),
        ] {
            assert!(record
                .verify_archive(&index, &"a".repeat(64), &fitting)
                .is_err());
        }
        let mut changed = index;
        changed["fitting_supported"] = json!(false);
        assert!(record
            .verify_archive(&changed, &"a".repeat(64), &legacy)
            .is_err());
    }
    #[test]
    fn every_bundled_fixture_has_verified_inventory_without_qualification_inference() {
        let library = crate::library::Library::embedded().unwrap();
        assert_eq!(library.records.len(), 8);
        assert_eq!(library.availability_counts["selected_model_identities"], 8);
        assert_eq!(library.availability_counts["selected_preset_requests"], 47);
        assert_eq!(library.availability_counts["selected_fit_ready"], 1);
        for record in &library.records {
            assert_eq!(record.metadata["availability_record_status"], "verified");
            assert_eq!(
                record.metadata["availability"]["current_engine_qualification"],
                "not_assessed"
            );
            assert_eq!(
                record.metadata["availability"]["publication_review"],
                "pending"
            );
            assert_eq!(
                record.metadata["availability"]["unit_status"],
                "source_scale_unqualified"
            );
        }
    }
}
