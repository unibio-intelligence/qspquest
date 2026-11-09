use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::Path,
    sync::OnceLock,
};

/// How the server treats a catalog archive that fails one of its own checks.
/// Library-wide checks (catalog, availability registry, duplicates) always
/// refuse the library. The server binary chooses with [`ArchiveChecks::for_profile`]:
/// holding is for the local profile only; public and managed deployments refuse a
/// library with a failing archive. `Hold` remains the default for embedded use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArchiveChecks {
    /// Withhold the failing archive, or only its fitting capability, report a
    /// structured hold and serve every other model.
    #[default]
    Hold,
    /// Refuse the whole library when any archive fails a check.
    Strict,
}

static CONFIGURED_CHECKS: OnceLock<ArchiveChecks> = OnceLock::new();

impl ArchiveChecks {
    /// Set the process-wide policy once, before the library is first loaded.
    /// Library refreshes use the same policy.
    pub fn configure(self) -> Result<(), String> {
        CONFIGURED_CHECKS
            .set(self)
            .map_err(|_| "Archive check policy is already configured".to_owned())
    }
    pub fn configured() -> Self {
        CONFIGURED_CHECKS.get().copied().unwrap_or_default()
    }
    /// The policy for a deployment profile: strict outside the local profile, so a checksum
    /// or unsafe-path failure stops a public or managed server at startup; holds locally
    /// unless strict checks are requested.
    pub fn for_profile(profile: crate::deployment::Profile, strict_requested: bool) -> Self {
        if strict_requested || profile != crate::deployment::Profile::Local {
            Self::Strict
        } else {
            Self::Hold
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HoldScope {
    /// The record serves catalog metadata only; it cannot be inspected as a
    /// native model, simulated or fitted.
    Archive,
    /// Fitting is withheld; inspection and simulation remain available.
    Fitting,
}

/// A model archive, or one of its capabilities, that this server withholds
/// because the archive failed a check at library load.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveHold {
    pub model_id: String,
    pub version: String,
    pub scope: HoldScope,
    /// Load stage that failed, for example `archive_integrity` or `fitting_evidence`.
    pub check: String,
    /// The failed condition, naming the differing fields where the check records them.
    pub message: String,
}

impl ArchiveHold {
    fn new(metadata: &Value, scope: HoldScope, check: &str, message: String) -> Self {
        Self {
            model_id: metadata["id"].as_str().unwrap_or("").to_owned(),
            version: metadata["version"].as_str().unwrap_or("").to_owned(),
            scope,
            check: check.to_owned(),
            message,
        }
    }
    fn effect(&self) -> String {
        let message = self.message.trim_end_matches('.');
        match self.scope {
            HoldScope::Archive => format!("This archive is withheld because it failed the {} check: {message}. Search lists its catalog metadata only; select another model to inspect or simulate.", self.check),
            HoldScope::Fitting => format!("Fitting is withheld because the archive failed the {} check: {message}. Inspection and stored or exploratory simulation remain available.", self.check),
        }
    }
    fn refusal(&self) -> String {
        format!(
            "Model {} {} failed the {} check: {}",
            self.model_id, self.version, self.check, self.message
        )
    }
    fn report(&self) {
        eprintln!("Library hold ({:?}): {}", self.scope, self.refusal());
    }
}

/// Withdraw the fitting claim from the record and its availability description.
fn withhold_fitting(metadata: &mut Value, hold: &ArchiveHold) {
    metadata["fit_ready"] = json!(false);
    metadata["fitting_supported"] = json!(false);
    metadata["archive_hold"] = json!(hold);
    if let Some(availability) = metadata
        .get_mut("availability")
        .and_then(Value::as_object_mut)
    {
        availability.insert("fit_ready".into(), json!(false));
        availability.insert("fitting_supported".into(), json!(false));
        if let Some(operations) = availability
            .get_mut("supported_operations")
            .and_then(Value::as_array_mut)
        {
            operations.retain(|operation| operation != "fit");
        }
        if let Some(blockers) = availability
            .get_mut("blockers")
            .and_then(Value::as_array_mut)
        {
            blockers.push(json!(format!("archive_hold_{}", hold.check)));
        }
    }
}

/// The catalog record that replaces an archive which failed a check.
fn held(
    mut metadata: Value,
    check: &str,
    message: String,
    checks: ArchiveChecks,
) -> Result<Record, String> {
    let hold = ArchiveHold::new(&metadata, HoldScope::Archive, check, message);
    if checks == ArchiveChecks::Strict {
        return Err(hold.refusal());
    }
    hold.report();
    let object = metadata
        .as_object_mut()
        .ok_or("Catalog metadata must be an object")?;
    // Executable declarations of a withheld archive are not served.
    for key in [
        "precision_profiles",
        "scientific_assessments",
        "protocol_initialization",
        "source_interpretations",
    ] {
        object.remove(key);
    }
    let declared = object.insert("scenario_ids".into(), json!([]));
    object.insert(
        "unavailable_scenario_ids".into(),
        declared.unwrap_or(json!([])),
    );
    object.insert("execution_available".into(), json!(false));
    object.insert("execution_blocker".into(), json!(hold.effect()));
    object.insert("fit_ready".into(), json!(false));
    object.insert("fitting_supported".into(), json!(false));
    object.insert("availability_record_status".into(), json!("held"));
    object.insert("archive_hold".into(), json!(hold));
    Ok(Record {
        metadata,
        model: Value::Null,
        index: json!({}),
        scenarios: BTreeMap::new(),
        archive_hash: String::new(),
        fitting: json!({"supported":false,"reason":hold.effect()}),
        source: None,
        precision_profiles: None,
    })
}

#[derive(Clone)]
pub struct Record {
    pub metadata: Value,
    pub model: Value,
    pub index: Value,
    pub scenarios: BTreeMap<String, Value>,
    pub archive_hash: String,
    pub fitting: Value,
    pub source: Option<String>,
    pub precision_profiles: Option<std::sync::Arc<crate::precision::Profiles>>,
}

#[cfg(test)]
mod catalog_contract_tests {
    use super::*;
    use std::io::Write;

    fn fixture(native: bool, fit_ready: bool, scenarios: Value) -> Result<Library, String> {
        fixture_with(ArchiveChecks::Strict, native, fit_ready, scenarios)
    }
    fn fixture_with(
        checks: ArchiveChecks,
        native: bool,
        fit_ready: bool,
        scenarios: Value,
    ) -> Result<Library, String> {
        let mut files = BTreeMap::new();
        if native {
            let model = crate::core::compute::execute(
                include_str!("../assets/templates/oral.qsp"),
                "validate_model",
                json!({}),
            )
            .unwrap()["model"]
                .clone();
            files.insert("native/model.json", serde_json::to_vec(&model).unwrap());
        }
        let members: serde_json::Map<String, Value> = files
            .iter()
            .map(|(name, bytes)| (name.to_string(), json!(hash(bytes))))
            .collect();
        let index = json!({"native_execution":{"model":if native {json!("native/model.json")} else {Value::Null}}, "members":members});
        files.insert("index.json", serde_json::to_vec(&index).unwrap());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let archive = zip.finish().unwrap().into_inner();
        let catalog =
            serde_json::to_vec(&json!({"models":[{"id":"synthetic_fixture", "version":"1",
            "archive":"fixture.omex", "omex_sha256":hash(&archive),
            "fit_ready":fit_ready, "fitting_supported":fit_ready, "scenario_ids":scenarios}]}))
            .unwrap();
        Library::load_with(checks, |path| match path {
            "catalog.json" => Ok(catalog.clone()),
            "fixture.omex" => Ok(archive.clone()),
            _ => Err("Unexpected fixture path".into()),
        })
    }

    #[test]
    fn indexed_native_archive_without_an_explicit_model_path_remains_runnable() {
        let mut files = BTreeMap::new();
        let model = crate::core::compute::execute(
            include_str!("../assets/templates/oral.qsp"),
            "validate_model",
            json!({}),
        )
        .unwrap()["model"]
            .clone();
        let raw = serde_json::to_vec(&model).unwrap();
        let index = json!({"native_execution":{},"members":{"native/model.json":hash(&raw)}});
        files.insert("native/model.json", raw);
        files.insert("index.json", serde_json::to_vec(&index).unwrap());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let archive = zip.finish().unwrap().into_inner();
        let catalog = serde_json::to_vec(&json!({"models":[{"id":"legacy","version":"1",
            "archive":"fixture.omex","omex_sha256":hash(&archive),"scenario_ids":[],
            "fit_ready":false,"fitting_supported":false}]}))
        .unwrap();
        let library = Library::load(|name| match name {
            "catalog.json" => Ok(catalog.clone()),
            "fixture.omex" => Ok(archive.clone()),
            _ => Err("Unexpected fixture path".into()),
        })
        .unwrap();
        assert_eq!(library.records[0].model, model);
        assert_eq!(library.records[0].fitting["supported"], false);
    }
    #[test]
    fn simulation_only_archive_does_not_gain_fitting_qualification() {
        let library = fixture(true, false, json!([])).unwrap();
        assert!(!library.records[0].model.is_null());
        assert_eq!(library.records[0].fitting["supported"], false);
        assert_eq!(library.records[0].metadata["fit_ready"], false);
    }

    #[test]
    fn held_archive_remains_discoverable_without_native_execution() {
        let library = fixture(false, false, json!([])).unwrap();
        assert!(library.records[0].model.is_null());
        assert!(library.records[0].scenarios.is_empty());
    }

    /// Strict loading refuses the library and names the model and check; the
    /// default withholds only the failing archive with the same reason.
    fn refused_and_held(native: bool, fit_ready: bool, scenarios: Value, reason: &str) {
        let error = fixture(native, fit_ready, scenarios.clone()).err().unwrap();
        assert!(error.starts_with("Model synthetic_fixture 1 failed the archive_integrity check:"));
        assert!(error.contains(reason), "{error}");
        let library =
            fixture_with(ArchiveChecks::Hold, native, fit_ready, scenarios.clone()).unwrap();
        let record = &library.records[0];
        let hold = &record.metadata["archive_hold"];
        assert_eq!(hold["model_id"], "synthetic_fixture");
        assert_eq!(hold["version"], "1");
        assert_eq!(hold["scope"], "archive");
        assert_eq!(hold["check"], "archive_integrity");
        assert!(hold["message"].as_str().unwrap().contains(reason));
        assert!(record.model.is_null() && record.scenarios.is_empty());
        assert_eq!(record.metadata["execution_available"], false);
        assert_eq!(record.metadata["fit_ready"], false);
        assert_eq!(record.metadata["scenario_ids"], json!([]));
        assert_eq!(record.metadata["unavailable_scenario_ids"], scenarios);
        assert!(record.metadata["execution_blocker"]
            .as_str()
            .unwrap()
            .contains(reason));
        assert_eq!(library.holds().len(), 1);
        let search = library.search("", &json!({}));
        assert_eq!(search["archive_holds"][0]["check"], "archive_integrity");
        assert_eq!(search["models"][0]["archive_hold"]["scope"], "archive");
    }

    #[test]
    fn fitting_claim_requires_its_hash_indexed_contract() {
        refused_and_held(true, true, json!([]), "fitting.json");
    }

    #[test]
    fn runnable_scenario_cannot_be_declared_without_a_native_model() {
        refused_and_held(false, false, json!(["unqualified"]), "native model");
    }
}
#[derive(Clone)]
pub struct Library {
    pub version: String,
    pub records: Vec<Record>,
    pub availability_counts: Value,
}

struct SearchRow {
    metadata: Value,
    index: Value,
    outputs: Value,
    text: String,
    executable: bool,
    user_source: bool,
    archive_hash: String,
    scenario_count: usize,
}
fn same_search_json(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            // JSON text distinguishes signed zero even when numeric equality does not.
            a == b && a.as_f64().map(f64::to_bits) == b.as_f64().map(f64::to_bits)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_search_json(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ak, av), (bk, bv))| ak == bk && same_search_json(av, bv))
        }
        _ => left == right,
    }
}
fn same_search_field(left: &Value, right: &Value, field: &str) -> bool {
    match (left.get(field), right.get(field)) {
        (Some(a), Some(b)) => same_search_json(a, b),
        (None, None) => true,
        _ => false,
    }
}
impl SearchRow {
    fn current(&self, record: &Record) -> bool {
        same_search_json(&self.metadata, &record.metadata)
            && same_search_json(&self.index, &record.index)
            && same_search_json(&self.outputs, &record.model["outputs"])
            && self.executable == !record.model.is_null()
            && self.user_source == record.source.is_some()
            && self.archive_hash == record.archive_hash
            && self.scenario_count == record.scenarios.len()
    }
    fn discovery_and_indices_current(&self, record: &Record, output_filter: bool) -> bool {
        [
            "id",
            "version",
            "license",
            "model_fidelity",
            "source_family",
        ]
        .into_iter()
        .all(|field| same_search_field(&self.metadata, &record.metadata, field))
            && self.executable == !record.model.is_null()
            && self.user_source == record.source.is_some()
            && self.archive_hash == record.archive_hash
            && self.scenario_count == record.scenarios.len()
            && (!output_filter
                || (same_search_json(&self.outputs, &record.model["outputs"])
                    && same_search_json(
                        &self.index["native_execution"]["outputs"],
                        &record.index["native_execution"]["outputs"],
                    )))
    }
}

/// One current catalog snapshot; library reloads replace its contents.
pub(crate) struct SearchIndex {
    version: String,
    prepared: bool,
    rows: Vec<SearchRow>,
    exact: BTreeMap<&'static str, BTreeMap<String, Vec<usize>>>,
    discovery: Result<Vec<crate::discovery::ModelDiscovery>, String>,
}
impl SearchIndex {
    fn uncached(library: &Library) -> Self {
        Self {
            version: hash(library.version.as_bytes()),
            prepared: false,
            rows: Vec::new(),
            exact: BTreeMap::new(),
            discovery: Ok(Vec::new()),
        }
    }
    pub(crate) fn new(library: &Library) -> Self {
        const MAX_RECORDS: usize = 4096;
        const MAX_INPUT_BYTES: usize = 8_000_000;
        if library.records.len() > MAX_RECORDS {
            return Self::uncached(library);
        }
        let mut input_bytes = library.version.len();
        if input_bytes > MAX_INPUT_BYTES {
            return Self::uncached(library);
        }
        let mut rows = Vec::new();
        for record in &library.records {
            let text = search_text(record);
            input_bytes = input_bytes
                .saturating_add(text.len())
                .saturating_add(record.model["outputs"].to_string().len())
                .saturating_add(record.archive_hash.len());
            if input_bytes > MAX_INPUT_BYTES {
                return Self::uncached(library);
            }
            rows.push(SearchRow {
                metadata: record.metadata.clone(),
                index: record.index.clone(),
                outputs: record.model["outputs"].clone(),
                text,
                executable: !record.model.is_null(),
                user_source: record.source.is_some(),
                archive_hash: record.archive_hash.clone(),
                scenario_count: record.scenarios.len(),
            });
        }
        let mut exact: BTreeMap<&'static str, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
        for (i, record) in library.records.iter().enumerate() {
            for (filter, field) in [
                ("model_id", "id"),
                ("version", "version"),
                ("license", "license"),
            ] {
                if let Some(value) = record.metadata[field].as_str() {
                    exact
                        .entry(filter)
                        .or_default()
                        .entry(value.to_owned())
                        .or_default()
                        .push(i);
                }
            }
            let mut outputs = std::collections::BTreeSet::new();
            if let Some(aliases) = record.index["native_execution"]["outputs"].as_object() {
                outputs.extend(aliases.keys().cloned());
            }
            if let Some(native) = record.model["outputs"].as_array() {
                outputs.extend(
                    native
                        .iter()
                        .filter_map(|o| o["name"].as_str().map(str::to_owned)),
                );
            }
            for output in outputs {
                exact
                    .entry("has_output")
                    .or_default()
                    .entry(output)
                    .or_default()
                    .push(i);
            }
        }
        Self {
            version: library.version.clone(),
            prepared: true,
            rows,
            exact,
            discovery: crate::discovery::describe_all(&library.records),
        }
    }
    pub(crate) fn current(&self, library: &Library) -> bool {
        if !self.prepared {
            return self.version == hash(library.version.as_bytes());
        }
        self.version == library.version
            && (self.rows.len() == library.records.len()
                && self
                    .rows
                    .iter()
                    .zip(&library.records)
                    .all(|(row, record)| row.current(record)))
    }
    fn candidates(&self, filters: &Value) -> Vec<usize> {
        let mut lists = Vec::new();
        for field in ["model_id", "version", "license", "has_output"] {
            if let Some(wanted) = filters[field].as_str() {
                let Some(indices) = self.exact.get(field).and_then(|values| values.get(wanted))
                else {
                    return Vec::new();
                };
                lists.push(indices);
            }
        }
        let Some(shortest) = lists.iter().min_by_key(|indices| indices.len()) else {
            return (0..self.rows.len()).collect();
        };
        shortest
            .iter()
            .copied()
            .filter(|i| lists.iter().all(|indices| indices.binary_search(i).is_ok()))
            .collect()
    }
    fn current_for_search(&self, library: &Library, query: &str, filters: &Value) -> bool {
        if !self.prepared {
            return self.current(library);
        }
        let output_filter = filters["has_output"].as_str().is_some();
        if self.version != library.version
            || self.rows.len() != library.records.len()
            || !self
                .rows
                .iter()
                .zip(&library.records)
                .all(|(row, record)| row.discovery_and_indices_current(record, output_filter))
        {
            return false;
        }
        if query.split_whitespace().next().is_some() || filters["tag"].as_str().is_some() {
            return self.candidates(filters).into_iter().all(|i| {
                same_search_json(&self.rows[i].metadata, &library.records[i].metadata)
                    && same_search_json(&self.rows[i].index, &library.records[i].index)
            });
        }
        true
    }
    pub(crate) fn search(&self, library: &Library, query: &str, filters: &Value) -> Option<Value> {
        self.current_for_search(library, query, filters)
            .then(|| library.search_with_index(query, filters, self.prepared.then_some(self)))
    }
}

#[cfg(test)]
mod archive_check_policy_tests {
    use super::ArchiveChecks;
    use crate::deployment::Profile;

    #[test]
    fn only_the_local_profile_holds_failing_archives() {
        assert_eq!(
            ArchiveChecks::for_profile(Profile::Local, false),
            ArchiveChecks::Hold
        );
        assert_eq!(
            ArchiveChecks::for_profile(Profile::Local, true),
            ArchiveChecks::Strict
        );
        assert_eq!(
            ArchiveChecks::for_profile(Profile::Public, false),
            ArchiveChecks::Strict
        );
        assert_eq!(
            ArchiveChecks::for_profile(Profile::Managed, false),
            ArchiveChecks::Strict
        );
    }
}

#[cfg(test)]
mod search_index_tests {
    use super::*;

    fn library() -> Library {
        let records = [
            ("beta", "dose clearance ÉTUDES", "A"),
            ("alpha", "dose", "B"),
            ("gamma", "clearance", "A"),
        ]
        .into_iter()
        .map(|(id, description, license)| Record {
            metadata: json!({"id":id,"version":"1","description":description,"license":license}),
            model: json!({"outputs":[{"name":"amount","unit":"mg"}]}),
            index: json!({"native_execution":{"outputs":{"exposure":"amount"}}}),
            scenarios: BTreeMap::new(),
            archive_hash: "a".repeat(64),
            fitting: Value::Null,
            source: None,
            precision_profiles: None,
        })
        .collect();
        Library {
            version: "v1".into(),
            records,
            availability_counts: Value::Null,
        }
    }
    fn ids(value: &Value) -> Vec<&str> {
        value["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect()
    }
    #[test]
    fn indexed_search_preserves_or_ranking_unicode_filters_and_page_totals() {
        let library = library();
        let index = SearchIndex::new(&library);
        let cases = [
            ("dose clearance", json!({})),
            ("DOSE", json!({"offset":1,"limit":1})),
            ("", json!({"tag":"études"})),
            ("", json!({"license":"A","has_output":"exposure"})),
            (
                "",
                json!({"has_output":"amount","model_id":"beta","version":"1"}),
            ),
            ("", json!({"model_id":"beta","version":"2"})),
            ("absent", json!({})),
            ("", json!({"offset":100,"limit":0})),
            ("", json!({"limit":10000})),
            ("", json!({"model_id":5,"has_output":false,"license":null})),
        ];
        for (query, filters) in cases {
            assert_eq!(
                index.search(&library, query, &filters).unwrap(),
                library.search(query, &filters)
            );
        }
        assert_eq!(
            ids(&index
                .search(&library, "dose clearance", &json!({}))
                .unwrap()),
            ["beta", "alpha", "gamma"]
        );
        let page = index
            .search(&library, "DOSE", &json!({"offset":1,"limit":1}))
            .unwrap();
        assert_eq!(ids(&page), ["alpha"]);
        assert_eq!(page["total"], 2);
        assert!(page["next_offset"].is_null());
        assert_eq!(
            ids(&index
                .search(&library, "", &json!({"tag":"études"}))
                .unwrap()),
            ["beta"]
        );
        assert_eq!(
            ids(&index
                .search(
                    &library,
                    "",
                    &json!({"license":"A","has_output":"exposure"})
                )
                .unwrap()),
            ["beta", "gamma"]
        );
    }
    #[test]
    fn every_search_input_invalidates_the_current_snapshot() {
        let original = library();
        let index = SearchIndex::new(&original);
        let mut variants = Vec::new();
        let mut changed = original.clone();
        changed.version = "v2".into();
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].metadata["description"] = json!("changed");
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].index["native_execution"]["outputs"] = json!({"renamed":"amount"});
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].model["outputs"][0]["unit"] = json!("g");
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].model = Value::Null;
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].source = Some("user model".into());
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].archive_hash = "b".repeat(64);
        variants.push(changed);
        let mut changed = original.clone();
        changed.records[0].scenarios.insert("new".into(), json!({}));
        variants.push(changed);
        let mut changed = original.clone();
        changed.records.swap(0, 1);
        variants.push(changed);
        let mut changed = original.clone();
        changed.records.pop();
        variants.push(changed);
        for changed in variants {
            assert!(!index.current(&changed));
            assert!(index
                .search(&changed, "dose", &json!({"has_output":"amount"}))
                .is_none());
            assert_eq!(
                SearchIndex::new(&changed)
                    .search(&changed, "", &json!({}))
                    .unwrap(),
                changed.search("", &json!({}))
            );
        }
        let mut unchanged = original.clone();
        unchanged.availability_counts = json!({"current":3});
        assert!(index.current(&unchanged));
        assert_eq!(
            index.search(&unchanged, "", &json!({})).unwrap()["availability_counts"],
            unchanged.availability_counts
        );
    }
    #[test]
    fn cloned_servers_rebuild_after_edits_and_refuse_invalid_unselected_families() {
        let server = crate::server::Server::new(library(), false, crate::policy::Tier::Free, 4);
        let cloned = server.clone();
        assert_eq!(
            ids(&server.library_search("dose", &json!({}))),
            ["beta", "alpha"]
        );
        {
            let mut library = server.library.write().unwrap();
            library.records[0].metadata["description"] = json!("newterm");
            library.records.swap(0, 2);
        }
        assert_eq!(ids(&cloned.library_search("newterm", &json!({}))), ["beta"]);
        assert_eq!(ids(&server.library_search("dose", &json!({}))), ["alpha"]);
        {
            let mut library = server.library.write().unwrap();
            *library = super::search_index_tests::library();
            library.version = "replacement".into();
        }
        assert_eq!(
            cloned.library_search("", &json!({}))["library_version"],
            "replacement"
        );
        server.library.write().unwrap().records[2].metadata["source_family"] =
            json!({"invalid":true});
        let error = cloned.library_search("", &json!({"model_id":"beta"}));
        assert_eq!(error["error"]["code"], "discovery_metadata");
        server.library.write().unwrap().records[2]
            .metadata
            .as_object_mut()
            .unwrap()
            .remove("source_family");
        assert_eq!(
            ids(&server.library_search("", &json!({"model_id":"beta"}))),
            ["beta"]
        );
        server.library.write().unwrap().records.clear();
        assert_eq!(cloned.library_search("", &json!({}))["total"], 0);
    }
    #[test]
    fn oversized_catalogs_fall_back_without_retaining_snapshots() {
        let mut library = library();
        let small = SearchIndex::new(&library);
        assert!(small.prepared);
        // Search text is curated (names, descriptions, scenarios, outputs), so pad a searched field.
        let original = library.records[0].metadata["description"].clone();
        library.records[0].metadata["description"] = json!("x".repeat(8_000_001));
        let large = SearchIndex::new(&library);
        assert!(!large.prepared);
        assert!(large.rows.is_empty());
        assert!(large.exact.is_empty());
        assert_eq!(
            large
                .search(&library, "", &json!({"model_id":"alpha"}))
                .unwrap(),
            library.search("", &json!({"model_id":"alpha"}))
        );
        library.records[0].metadata["description"] = original;
        library.records[1].metadata["description"] = json!("newterm");
        assert_eq!(
            large.search(&library, "newterm", &json!({})).unwrap()["total"],
            1
        );
        library.records = vec![library.records[1].clone(); 4097];
        let many = SearchIndex::new(&library);
        assert!(!many.prepared);
        assert!(many.rows.is_empty());
        assert_eq!(
            many.search(&library, "", &json!({"limit":1})).unwrap()["total"],
            4097
        );
    }
    #[test]
    fn numeric_text_changes_invalidate_even_for_equal_signed_zero_values() {
        let mut library = library();
        library.records[0].metadata["numeric_annotation"] = json!(0.0);
        let index = SearchIndex::new(&library);
        let server =
            crate::server::Server::new(library.clone(), false, crate::policy::Tier::Free, 4);
        library.records[0].metadata["numeric_annotation"] = json!(-0.0);
        assert!(!index.current(&library));
        server.library.write().unwrap().records[0].metadata["numeric_annotation"] = json!(-0.0);
        assert_eq!(
            server.library_search("", &json!({})),
            library.search("", &json!({}))
        );
        // A change to searched text after the index was built reaches the results: the stale
        // index is not used, and cached and uncached searches agree.
        assert_eq!(
            server.library_search("recalibrated", &json!({}))["total"],
            0
        );
        server.library.write().unwrap().records[0].metadata["description"] =
            json!("recalibrated assay");
        library.records[0].metadata["description"] = json!("recalibrated assay");
        assert_eq!(
            ids(&server.library_search("recalibrated", &json!({}))),
            ["beta"]
        );
        assert_eq!(
            server.library_search("recalibrated", &json!({})),
            library.search("recalibrated", &json!({}))
        );
    }
    #[test]
    fn query_scoped_validation_preserves_live_metadata_and_null_presence() {
        let mut library = library();
        let index = SearchIndex::new(&library);
        library.records[0].metadata["description"] = json!("newterm");
        let exact = json!({"model_id":"beta"});
        let value = index.search(&library, "", &exact).unwrap();
        assert_eq!(value["models"][0]["description"], "newterm");
        assert_eq!(value, library.search("", &exact));
        assert!(index.search(&library, "newterm", &json!({})).is_none());
        let excluded = json!({"model_id":"alpha"});
        assert_eq!(
            index.search(&library, "newterm", &excluded).unwrap(),
            library.search("newterm", &excluded)
        );
        for field in ["model_fidelity", "source_family"] {
            let mut changed = library.clone();
            changed.records[2].metadata[field] = Value::Null;
            assert!(index.search(&changed, "", &exact).is_none());
            assert_eq!(
                SearchIndex::new(&changed)
                    .search(&changed, "", &exact)
                    .unwrap()["error"]["code"],
                "discovery_metadata"
            );
        }
    }
}
pub trait ModelSource {
    fn find(&self, id: &str, version: Option<&str>) -> Option<&Record>;
}
impl Record {
    pub fn source_text(&self) -> String {
        if self.model.is_null() {
            return self.metadata.to_string();
        }
        self.source
            .clone()
            .unwrap_or_else(|| self.model.to_string())
    }
}
impl ModelSource for Library {
    fn find(&self, id: &str, version: Option<&str>) -> Option<&Record> {
        self.records
            .iter()
            .find(|r| r.metadata["id"] == id && version.is_none_or(|v| r.metadata["version"] == v))
    }
}
pub fn display_states(model: &Value) -> Vec<Value> {
    let mut states: Vec<Value> = model["states"].as_array().map(|rows| rows.iter().map(|st|json!({"name":st["name"],"unit":st["unit"],"initial":st["initial"],"dosing":st["dosing"]})).collect()).unwrap_or_default();
    if let Some(pk) = model.get("linear_pk").filter(|p| !p.is_null()) {
        let mut rows = Vec::new();
        if let Some(depot) = pk.get("depot").filter(|d| !d.is_null()) {
            rows.push(depot);
        }
        if let Some(compartments) = pk["compartments"].as_array() {
            rows.extend(compartments);
        }
        for row in rows {
            let dosing = row
                .get("dosing_override")
                .filter(|d| !d.is_null())
                .cloned()
                .unwrap_or_else(|| {
                    json!({"amount_unit":pk["amount_unit"],"modes":["bolus","infusion"],
                        "scale":qspquest_core::expression::Expr::number(1.),"lag":null})
                });
            states.push(json!({"name":row["state"],"unit":pk["amount_unit"],"initial":row["initial"],"dosing":dosing}));
        }
    }
    states
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn zip_json(z: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<Value, String> {
    let f = z.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    if f.size() > 4_000_000 {
        return Err("Archive member exceeds 4 MB".into());
    }
    let mut b = Vec::new();
    f.take(4_000_001)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}
impl Library {
    pub(crate) fn verified_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Self, String> {
        Self::load(|path| {
            files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("Missing release member: {path}"))
        })
    }
    /// The synthetic library used by this crate's unit tests.
    #[cfg(test)]
    pub(crate) fn fixture() -> Result<Self, String> {
        Self::directory(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/library"
        )))
    }
    /// Load a library directory under the configured archive check policy.
    pub fn directory(dir: &Path) -> Result<Self, String> {
        Self::directory_with(dir, ArchiveChecks::configured())
    }
    pub fn directory_with(dir: &Path, checks: ArchiveChecks) -> Result<Self, String> {
        if !dir.join("catalog.json").is_file() {
            return Self::source_directory(dir);
        }
        Self::load_with(checks, |path| {
            let rel = Path::new(path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("Unsafe library path".into());
            }
            let p = dir.join(rel);
            let canon = p.canonicalize().map_err(|e| e.to_string())?;
            if !canon.starts_with(dir.canonicalize().map_err(|e| e.to_string())?) {
                return Err("Library path escapes directory".into());
            }
            {
                let file = std::fs::File::open(canon).map_err(|e| e.to_string())?;
                if file.metadata().map_err(|e| e.to_string())?.len() > 16_000_000 {
                    return Err("Library file exceeds 16 MB".into());
                }
                let mut bytes = Vec::new();
                file.take(16_000_001)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                Ok(bytes)
            }
        })
    }
    fn load(read: impl Fn(&str) -> Result<Vec<u8>, String>) -> Result<Self, String> {
        Self::load_with(ArchiveChecks::configured(), read)
    }
    fn load_with(
        checks: ArchiveChecks,
        read: impl Fn(&str) -> Result<Vec<u8>, String>,
    ) -> Result<Self, String> {
        let bytes = read("catalog.json")?;
        if bytes.len() > 1_000_000 {
            return Err("Catalog exceeds 1 MB".into());
        }
        let catalog: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let availability = if let Some(schema) = catalog.get("availability_schema") {
            if schema != crate::availability::SCHEMA {
                return Err("Unsupported catalog availability schema".into());
            }
            let registry_bytes = read("qspquest_models/availability.json")?;
            if let Some(expected) = catalog.get("availability_sha256") {
                if expected.as_str() != Some(hash(&registry_bytes).as_str()) {
                    return Err("Availability registry checksum differs from catalog".into());
                }
            }
            Some(crate::availability::Registry::parse(
                &registry_bytes,
                &catalog,
            )?)
        } else {
            if catalog.get("availability_sha256").is_some() {
                return Err("Availability checksum requires its declared schema".into());
            }
            None
        };
        let availability_counts = availability
            .as_ref()
            .map_or(Value::Null, crate::availability::Registry::counts);
        let mut records = Vec::new();
        for metadata in catalog["models"]
            .as_array()
            .ok_or("catalog models missing")?
        {
            let mut metadata = metadata.clone();
            if records.len() >= 1024 {
                return Err("Catalog exceeds 1024 model versions".into());
            }
            if records.iter().any(|r: &Record| {
                r.metadata["id"] == metadata["id"] && r.metadata["version"] == metadata["version"]
            }) {
                return Err("Duplicate catalog model/version".into());
            }
            metadata
                .as_object_mut()
                .ok_or("Catalog metadata must be an object")?
                .remove("scientific_assessment_contract");
            if metadata["format"] == "metadata_only" {
                if metadata.get("archive").is_some()
                    || metadata.get("precision_profiles").is_some()
                    || metadata.get("scientific_assessments").is_some()
                    || metadata.get("source_interpretations").is_some()
                    || metadata["fit_ready"] == true
                    || metadata["fitting_supported"] == true
                {
                    return Err("Metadata-only record must not declare executable content".into());
                }
                let mut metadata = metadata.clone();
                metadata["execution_available"] = json!(false);
                metadata["availability_record_status"] = json!("unavailable");
                records.push(Record {
                    metadata,
                    model: Value::Null,
                    index: json!({}),
                    scenarios: BTreeMap::new(),
                    archive_hash: String::new(),
                    fitting: json!({}),
                    source: None,
                    precision_profiles: None,
                });
                continue;
            }
            let mut check = "archive_integrity";
            match archive_record(
                metadata.clone(),
                &read,
                availability.as_ref(),
                checks,
                &mut check,
            ) {
                Ok(record) => records.push(record),
                Err(message) => records.push(held(metadata, check, message, checks)?),
            }
        }
        crate::discovery::validate(&records)?;
        // Registry counts describe the archives as published; holds applied at load can withdraw
        // fitting, so count what this server will actually serve.
        let mut availability_counts = availability_counts;
        if availability_counts.is_object() {
            availability_counts["selected_fit_ready"] = json!(records
                .iter()
                .filter(|r| r.metadata["fit_ready"] == true)
                .count());
        }
        Ok(Self {
            version: hash(&bytes),
            records,
            availability_counts,
        })
    }
    /// Archives, or archive capabilities, withheld at load.
    pub fn holds(&self) -> Vec<ArchiveHold> {
        self.records
            .iter()
            .filter_map(|r| r.metadata.get("archive_hold"))
            .filter_map(|hold| serde_json::from_value(hold.clone()).ok())
            .collect()
    }
    pub fn search(&self, query: &str, filters: &Value) -> Value {
        self.search_with_index(query, filters, None)
    }
    fn search_with_index(
        &self,
        query: &str,
        filters: &Value,
        index: Option<&SearchIndex>,
    ) -> Value {
        let discovery = match index {
            Some(index) => match &index.discovery {
                Ok(records) => std::borrow::Cow::Borrowed(records.as_slice()),
                Err(message) => return crate::policy::error("discovery_metadata", message.clone()),
            },
            None => match crate::discovery::describe_all(&self.records) {
                Ok(records) => std::borrow::Cow::Owned(records),
                Err(message) => return crate::policy::error("discovery_metadata", message),
            },
        };
        let tokens: Vec<String> = query_words(query);
        let tag = filters["tag"].as_str().map(str::to_lowercase);
        let candidates = index.map_or_else(
            || (0..self.records.len()).collect(),
            |index| index.candidates(filters),
        );
        let mut hits: Vec<_> = candidates
            .into_iter()
            .filter_map(|i| {
                let r = &self.records[i];
                for field in ["model_id", "version"] {
                    if let Some(wanted) = filters[field].as_str() {
                        let key = if field == "model_id" { "id" } else { "version" };
                        if r.metadata[key].as_str() != Some(wanted) {
                            return None;
                        }
                    }
                }
                if let Some(license) = filters["license"].as_str() {
                    if r.metadata["license"] != license {
                        return None;
                    }
                }
                if let Some(output) = filters["has_output"].as_str() {
                    if !r.index["native_execution"]["outputs"]
                        .as_object()
                        .is_some_and(|m| m.contains_key(output))
                        && !r.model["outputs"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|o| o["name"] == output))
                    {
                        return None;
                    }
                }
                let (score, matched_terms) = if tokens.is_empty() && tag.is_none() {
                    (0, Vec::new())
                } else {
                    let text = index.map_or_else(
                        || std::borrow::Cow::Owned(search_text(r)),
                        |index| std::borrow::Cow::Borrowed(index.rows[i].text.as_str()),
                    );
                    if tag.as_ref().is_some_and(|tag| !text.contains(tag)) {
                        return None;
                    }
                    let (score, matched) = search_score(&tokens, r);
                    if !tokens.is_empty() && matched.is_empty() {
                        return None;
                    }
                    (score, matched)
                };
                if !search_filters_match(r, &discovery[i], filters) {
                    return None;
                }
                Some((score, i, matched_terms))
            })
            .collect();
        hits.sort_by(|a, b| (b.2.len(), b.0).cmp(&(a.2.len(), a.0)));
        let total = hits.len();
        let offset = filters["offset"].as_u64().unwrap_or(0) as usize;
        let limit = filters["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
        let models = hits
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, i, matched)| {
                let r = &self.records[i];
                let discovery = &discovery[i];
                let mut m = r.metadata.clone();
                if !matched.is_empty() {
                    m["matched_terms"] = json!(matched);
                }
                m["execution_available"] = json!(discovery.execution_available);
                m["discovery"] = json!(discovery);
                m["outputs"] = json!(r.model["outputs"].as_array().map(|a| {
                    a.iter()
                        .take(16)
                        .map(|o| json!({"name":o["name"],"unit":o["unit"]}))
                        .collect::<Vec<_>>()
                }));
                m["output_count"] = json!(r.model["outputs"].as_array().map_or(0, Vec::len));
                m["output_aliases"] = r.index["native_execution"]["outputs"].clone();
                m["scenario_count"] = json!(r.scenarios.len());
                m
            })
            .collect::<Vec<_>>();
        let next = offset.checked_add(models.len()).filter(|n| *n < total);
        let mut result = json!({"library_version":self.version,"availability_counts":self.availability_counts,"models":models,"total":total,"offset":offset,"next_offset":next});
        let holds = self.holds();
        if !holds.is_empty() {
            result["archive_holds"] = json!(holds);
        }
        result
    }
}

/// Lowercase words of a text after joining domain phrases into single concept words, so that
/// "target-mediated drug disposition" and "TMDD" (or "antibody-drug conjugate" and "ADC")
/// match each other. Hashes, paths and archive bookkeeping are never part of searched text.
/// Query words: phrases joined as in model text, and plain synonyms mapped to their concept,
/// without the derived concepts (a drug name ending in -mab must still match only that drug).
fn query_words(query: &str) -> Vec<String> {
    // Generic words carry no meaning for ranking ("anti-CD20 model for B cells").
    const IGNORED: [&str; 16] = [
        "anti", "model", "models", "for", "of", "the", "a", "an", "in", "and", "or", "with", "to",
        "on", "by", "any",
    ];
    let words = search_words(query);
    let base = words.len() - derived_concepts(&words_only(query)).len();
    words
        .into_iter()
        .take(base)
        .filter(|w| !IGNORED.contains(&w.as_str()))
        .map(|w| synonym(&w).unwrap_or(&w).to_owned())
        .collect()
}
fn words_only(text: &str) -> Vec<String> {
    let mut words = search_words(text);
    let derived = derived_concepts(&words).len();
    words.truncate(words.len() - derived);
    words
}
fn synonym(word: &str) -> Option<&'static str> {
    Some(match word {
        "mice" | "murine" | "mouse" => "mouse",
        "rats" | "rat" => "rat",
        "monkey" | "monkeys" | "cynomolgus" | "nhp" => "monkey",
        "pharmacokinetic" | "pharmacokinetics" => "pk",
        "pharmacodynamic" | "pharmacodynamics" => "pd",
        "bite" | "bispecific" | "bispecifics" | "bsab" => "bispecific",
        "antibodies" | "mab" | "mabs" | "immunoglobulin" | "igg" => "antibody",
        _ => return None,
    })
}
fn derived_concepts(words: &[String]) -> Vec<String> {
    words
        .iter()
        .filter_map(|w| {
            synonym(w).or(if w.len() > 4 && w.ends_with("mab") {
                Some("antibody")
            } else {
                None
            })
        })
        .map(str::to_owned)
        .collect()
}

pub(crate) fn search_words(text: &str) -> Vec<String> {
    let mut text = format!(
        " {} ",
        text.to_lowercase()
            .replace(['-', '_', '/', '(', ')', ',', '.', ';', ':'], " ")
    );
    for (phrase, concept) in [
        (" target mediated drug disposition ", " tmdd "),
        (" target mediated ", " tmdd "),
        (" antibody drug conjugates ", " adc "),
        (" antibody drug conjugate ", " adc "),
        (" cytokine release syndrome ", " crs "),
        (" cytokine release ", " crs "),
        (" b cells ", " bcell "),
        (" b cell ", " bcell "),
        (" t cells ", " tcell "),
        (" t cell ", " tcell "),
        (" population pharmacokinetic ", " poppk "),
        (" population pk ", " poppk "),
        (" two compartment ", " two_compartment "),
        (" one compartment ", " one_compartment "),
    ] {
        while text.contains(phrase) {
            text = text.replace(phrase, concept);
        }
    }
    let mut words: Vec<String> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let extra = derived_concepts(&words);
    words.extend(extra);
    words
}

/// Fields searched for a record, from most to least specific: identity, description, then
/// scenario, output and tag vocabulary. Archive indexes, digests and paths are excluded.
fn search_fields(r: &Record) -> [(String, &'static str, usize); 3] {
    let m = &r.metadata;
    let head = format!(
        "{} {}",
        m["id"].as_str().unwrap_or(""),
        m["name"].as_str().unwrap_or("")
    );
    let mut description = m["description"].as_str().unwrap_or("").to_owned();
    for key in [
        "descriptive_tags",
        "tags",
        "drugs",
        "targets",
        "species",
        "routes",
        "modality",
        "indication",
    ] {
        match &m[key] {
            Value::String(s) => description.push_str(&format!(" {s}")),
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .for_each(|s| description.push_str(&format!(" {s}"))),
            _ => {}
        }
    }
    if let Some(title) = m["source_citation"]["title"].as_str() {
        description.push_str(&format!(" {title}"));
    }
    let mut other = String::new();
    for scenario in m["scenario_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        other.push_str(&format!(" {scenario}"));
    }
    for output in r.model["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o["name"].as_str())
    {
        other.push_str(&format!(" {output}"));
    }
    for alias in r.index["native_execution"]["outputs"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        other.push_str(&format!(" {alias}"));
    }
    [
        (head, "name", 4),
        (description, "description", 2),
        (other, "scenarios_or_outputs", 1),
    ]
}

fn search_text(r: &Record) -> String {
    search_fields(r)
        .iter()
        .map(|(text, _, _)| text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Score whole-word matches (prefix matches for tokens of five or more characters, and simple
/// plurals). Returns the score and the matched terms with the field where each matched.
fn search_score(tokens: &[String], r: &Record) -> (usize, Vec<Value>) {
    let fields: Vec<(Vec<String>, &str, usize)> = search_fields(r)
        .into_iter()
        .map(|(text, name, weight)| (search_words(&text), name, weight))
        .collect();
    let mut score = 0;
    let mut matched = Vec::new();
    for token in tokens {
        let hit = |w: &String| {
            w == token
                || (token.len() >= 5 && w.starts_with(token.as_str()))
                || w.strip_suffix('s') == Some(token.as_str())
                || token.strip_suffix('s') == Some(w.as_str())
        };
        if let Some((_, field, weight)) = fields.iter().find(|(words, _, _)| words.iter().any(hit))
        {
            score += weight;
            matched.push(json!({"term":token,"field":field}));
        }
    }
    (score, matched)
}

/// Typed search filters beyond exact identity: availability, readiness and unit status.
fn search_filters_match(
    r: &Record,
    discovery: &crate::discovery::ModelDiscovery,
    filters: &Value,
) -> bool {
    let availability = &r.metadata["availability"];
    if let Some(wanted) = filters["availability"].as_str() {
        if availability["availability"].as_str() != Some(wanted) {
            return false;
        }
    }
    if let Some(wanted) = filters["unit_status"].as_str() {
        if availability["unit_status"].as_str() != Some(wanted) {
            return false;
        }
    }
    for key in ["fit_ready", "fitting_supported"] {
        if let Some(wanted) = filters[key].as_bool() {
            if r.metadata[key].as_bool().unwrap_or(false) != wanted {
                return false;
            }
        }
    }
    if let Some(wanted) = filters["execution_available"].as_bool() {
        if discovery.execution_available != crate::discovery::ExecutionAvailability::Known(wanted) {
            return false;
        }
    }
    if let Some(wanted) = filters["supports"].as_str() {
        if !availability["supported_operations"]
            .as_array()
            .is_some_and(|ops| ops.iter().any(|o| o == wanted))
        {
            return false;
        }
    }
    true
}

fn archive_record(
    mut metadata: Value,
    read: &impl Fn(&str) -> Result<Vec<u8>, String>,
    availability: Option<&crate::availability::Registry>,
    checks: ArchiveChecks,
    check: &mut &'static str,
) -> Result<Record, String> {
    let archive = read(metadata["archive"].as_str().ok_or("archive path missing")?)?;
    if archive.len() > 16_000_000 {
        return Err("Archive exceeds 16 MB".into());
    }
    let archive_hash = hash(&archive);
    if metadata["omex_sha256"].as_str() != Some(archive_hash.as_str()) {
        return Err(format!("Archive checksum failed for {}", metadata["id"]));
    }
    let mut z = zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| e.to_string())?;
    let mut names = std::collections::BTreeSet::new();
    let mut expanded = 0u64;
    if z.len() > 256 {
        return Err("Too many archive members".into());
    }
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|e| e.to_string())?;
        let path = Path::new(f.name());
        if !names.insert(f.name().to_owned())
            || path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || f.name().contains('\\')
        {
            return Err("Duplicate or unsafe archive member path".into());
        }
        expanded = expanded
            .checked_add(f.size())
            .ok_or("Archive size overflow")?;
        if expanded > 64_000_000 {
            return Err("Expanded archive exceeds 64 MB".into());
        }
    }
    let index = zip_json(&mut z, "index.json")?;
    let inspection_only = index["native_execution"].get("model") == Some(&Value::Null);
    if inspection_only
        && (metadata["scenario_ids"]
            .as_array()
            .is_none_or(|a| !a.is_empty())
            || metadata["fit_ready"] == true
            || metadata["fitting_supported"] == true
            || index["native_execution"]["scenarios"]
                .as_object()
                .is_some_and(|a| !a.is_empty()))
    {
        return Err("Inspection-only archive without a native model must not declare executable scenarios or fitting support".into());
    }
    let native_model = index["native_execution"]["model"].as_str().or_else(|| {
        // Earlier native archives omit the path but hash-index this member.
        (index["native_execution"].get("model").is_none()
            && index["members"].get("native/model.json").is_some())
        .then_some("native/model.json")
    });
    let scenarios = metadata["scenario_ids"]
        .as_array()
        .ok_or("scenario_ids missing")?;
    if native_model.is_none() && !scenarios.is_empty() {
        return Err("Runnable scenarios require a native model".into());
    }
    let has_fitting = index["members"]
        .as_object()
        .is_some_and(|m| m.contains_key("fitting.json"));
    if (metadata["fit_ready"] == true || metadata["fitting_supported"] == true) && !has_fitting {
        return Err("Fitting-enabled archive must hash-index fitting.json".into());
    }
    for path in native_model
        .into_iter()
        .chain(has_fitting.then_some("fitting.json"))
        .map(str::to_owned)
        .chain(
            metadata["scenario_ids"]
                .as_array()
                .ok_or("scenario_ids missing")?
                .iter()
                .filter_map(Value::as_str)
                .map(|id| format!("native/scenarios/{id}.json")),
        )
    {
        if !index["members"]
            .as_object()
            .is_some_and(|m| m.contains_key(&path))
        {
            return Err(format!(
                "Required native member is not hash-indexed: {path}"
            ));
        }
    }
    for (path, digest) in index["members"]
        .as_object()
        .ok_or("index members missing")?
    {
        let f = z.by_name(path).map_err(|e| e.to_string())?;
        if f.size() > 4_000_000 {
            return Err("Archive member too large".into());
        }
        let mut b = Vec::new();
        f.take(4_000_001)
            .read_to_end(&mut b)
            .map_err(|e| e.to_string())?;
        if digest.as_str() != Some(hash(&b).as_str()) {
            return Err(format!("Member checksum failed: {path}"));
        }
    }
    *check = "availability_record";
    if let Some(registry) = availability {
        let record = registry.for_model(&metadata)?;
        let fitting = if has_fitting {
            zip_json(&mut z, "fitting.json")?
        } else {
            json!({})
        };
        record.verify_archive(&index, &archive_hash, &fitting)?;
        let evidence_path = Path::new(metadata["archive"].as_str().ok_or("Archive path missing")?)
            .with_file_name("evidence.json")
            .to_string_lossy()
            .into_owned();
        if !record.evidence_matches(&read(&evidence_path)?) {
            return Err("Availability evidence digest mismatch".into());
        }
        metadata["availability"] = record.describe();
        metadata["availability_record_status"] = json!("verified");
    } else {
        metadata["availability_record_status"] = json!("unavailable");
    }
    metadata["execution_available"] = json!(native_model.is_some());
    *check = "archive_declarations";
    if inspection_only {
        if metadata.get("source_interpretations").is_some() {
            return Err(
                "Inspection-only archives cannot declare native structural interpretations".into(),
            );
        }
        if metadata.get("scientific_assessments").is_some() {
            return Err(
                "Inspection-only archives cannot declare native scientific assessments".into(),
            );
        }
        if metadata.get("protocol_initialization").is_some() {
            return Err(
                "Inspection-only archives must not declare executable protocol initialization"
                    .into(),
            );
        }
        if metadata.get("precision_profiles").is_some() {
            return Err(
                "Inspection-only archives must not declare executable precision profiles".into(),
            );
        }
        let mut held = metadata.clone();
        held["execution_available"] = json!(false);
        held["execution_blocker"] = json!("This archive retains source material without a qualified native projection. Select another runnable model; source-only scenarios cannot be simulated.");
        return Ok(Record {
            metadata: held,
            model: Value::Null,
            index,
            scenarios: BTreeMap::new(),
            archive_hash,
            fitting: json!({"supported":false}),
            source: None,
            precision_profiles: None,
        });
    }
    *check = "native_model";
    let model = if let Some(path) = native_model {
        zip_json(&mut z, path)?
    } else {
        Value::Null
    };
    *check = "source_interpretations";
    crate::interpretations::validate(&metadata, &model, &index, &archive_hash)?;
    *check = "fitting_evidence";
    let mut fitting = if has_fitting {
        zip_json(&mut z, "fitting.json")?
    } else {
        json!({"supported":false,"reason":"This archive supplies simulation scenarios, without fitting scenarios."})
    };
    if let Err(message) =
        crate::fitting::validate_archive(&fitting, &metadata, &model, &index, |path| {
            let mut file = z.by_name(path).map_err(|e| e.to_string())?;
            if file.size() > 1_000_000 {
                return Err("Fitting data/evidence exceeds 1 MB".into());
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            Ok(bytes)
        })
    {
        if checks == ArchiveChecks::Strict {
            return Err(message);
        }
        // Fitting rests on its own recorded evidence; inspection and
        // simulation of the same archive do not depend on it.
        let hold = ArchiveHold::new(&metadata, HoldScope::Fitting, check, message);
        hold.report();
        fitting = json!({"supported":false,"reason":hold.effect(),"archive_hold":hold});
        withhold_fitting(&mut metadata, &hold);
    }
    *check = "stored_scenarios";
    let mut scenarios = BTreeMap::new();
    for id in metadata["scenario_ids"]
        .as_array()
        .ok_or("scenario_ids missing")?
    {
        let id = id.as_str().ok_or("scenario id must be text")?;
        scenarios.insert(
            id.into(),
            zip_json(&mut z, &format!("native/scenarios/{id}.json"))?,
        );
    }
    *check = "precision_profiles";
    let precision_profiles = if let Some(descriptor) = metadata.get("precision_profiles") {
        let descriptor: crate::precision::Descriptor =
            serde_json::from_value(descriptor.clone())
                .map_err(|e| format!("Precision profile descriptor: {e}"))?;
        if descriptor.path.contains(['\\', ':'])
            || descriptor
                .path
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
        {
            return Err("Unsafe precision profile path".into());
        }
        let native_hash = native_model
            .and_then(|p| index["members"][p].as_str())
            .ok_or("Precision profiles require a hash-indexed native model")?;
        Some(std::sync::Arc::new(crate::precision::Profiles::load(
            &read(&descriptor.path)?,
            &descriptor,
            &metadata,
            &model,
            native_hash,
            &scenarios,
        )?))
    } else {
        None
    };
    *check = "scientific_assessments";
    if let Some(descriptor) = metadata.get("scientific_assessments") {
        let descriptor: crate::scientific::Descriptor = serde_json::from_value(descriptor.clone())
            .map_err(|e| format!("Scientific assessment descriptor: {e}"))?;
        if descriptor.path.contains(['\\', ':'])
            || descriptor
                .path
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
        {
            return Err("Unsafe scientific assessment path".into());
        }
        let native_hash = native_model
            .and_then(|p| index["members"][p].as_str())
            .ok_or("Scientific assessments require a hash-indexed native model")?;
        let declaration = crate::scientific::Declaration::load(
            &read(&descriptor.path)?,
            &descriptor,
            &metadata,
            &model,
            native_hash,
        )?;
        metadata["scientific_assessment_contract"] = json!(declaration);
    }
    *check = "protocol_initialization";
    if let Some(descriptor) = metadata.get("protocol_initialization") {
        let descriptor: crate::initialization::Descriptor =
            serde_json::from_value(descriptor.clone())
                .map_err(|e| format!("Protocol initialization descriptor: {e}"))?;
        if descriptor.path.contains(['\\', ':'])
            || descriptor
                .path
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
        {
            return Err("Unsafe protocol initialization path".into());
        }
        let native_hash = native_model
            .and_then(|p| index["members"][p].as_str())
            .ok_or("Protocol initialization requires a hash-indexed native model")?;
        let declaration = crate::initialization::Declaration::load(
            &read(&descriptor.path)?,
            &descriptor,
            &metadata,
            &model,
            &index,
            &scenarios,
            native_hash,
        )?;
        metadata["protocol_initialization_contract"] = json!(declaration);
    }
    if let Some(sidecar) = parameter_metadata(&metadata, &archive_hash, read) {
        metadata["parameter_metadata"] = sidecar;
    }
    Ok(Record {
        metadata: metadata.clone(),
        model,
        index,
        scenarios,
        archive_hash,
        fitting,
        source: None,
        precision_profiles,
    })
}

/// Optional `parameters.json` beside an archive: readable names, roles, variability and
/// provenance (`qspquest.models.parameter-metadata/v1`). It is not checksum-pinned; it is used
/// only when it names this exact archive, and a missing or mismatched sidecar means "no
/// metadata", never a load failure.
fn parameter_metadata(
    metadata: &Value,
    archive_hash: &str,
    read: &impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Option<Value> {
    let path = Path::new(metadata["archive"].as_str()?)
        .with_file_name("parameters.json")
        .to_string_lossy()
        .into_owned();
    let raw = read(&path).ok()?;
    if raw.len() > 1_000_000 {
        eprintln!("Ignoring {path}: parameter metadata exceeds 1 MB");
        return None;
    }
    let value: Value = serde_json::from_slice(&raw).ok()?;
    let matches = value["schema"] == "qspquest.models.parameter-metadata/v1"
        && value["model_id"] == metadata["id"]
        && value["archive_sha256"].as_str() == Some(archive_hash);
    if !matches {
        eprintln!("Ignoring {path}: parameter metadata does not describe this archive");
        return None;
    }
    Some(value)
}

/// A candidate record for one user-supplied `.qsp` or model JSON source, as served from a
/// source directory. It carries no stored scenarios, archive evidence or qualification.
pub fn source_record(source: &str) -> Result<Record, String> {
    let document = crate::core::compute::model(source).map_err(|e| e.to_string())?;
    let id = document.model_id.clone();
    let model = serde_json::to_value(document).map_err(|e| e.to_string())?;
    let digest = hash(source.as_bytes());
    Ok(Record {
        metadata: json!({"id":id,"version":digest,"name":id,"description":"User directory candidate; scientific qualification and redistribution rights are not assumed","license":"user supplied","candidate":true,"availability_record_status":"unavailable","execution_available":true,"fit_ready":false,"fitting_supported":false,"scenario_ids":[]}),
        model,
        index: json!({}),
        scenarios: BTreeMap::new(),
        archive_hash: digest,
        fitting: json!({}),
        source: Some(source.to_owned()),
        precision_profiles: None,
    })
}

impl Library {
    /// A one-record library for inspecting an inline source with the ordinary card builders.
    pub fn single(record: Record) -> Self {
        Self {
            version: hash(record.metadata.to_string().as_bytes()),
            records: vec![record],
            availability_counts: Value::Null,
        }
    }
    fn source_directory(dir: &Path) -> Result<Self, String> {
        let mut records = Vec::new();
        let mut entries = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        if entries.len() > 256 {
            return Err("Directory has more than 256 entries".into());
        }
        for entry in entries {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Invalid filename")?;
            if name.ends_with(".request.json")
                || name.ends_with(".scenarios.json")
                || !matches!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("qsp" | "json")
                )
            {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > 1_000_000 {
                return Err(
                    "Model sources must be regular files under 1 MB; symlinks are forbidden".into(),
                );
            }
            let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut record = source_record(&source)?;
            if records
                .iter()
                .any(|r: &Record| r.metadata["id"] == record.metadata["id"])
            {
                return Err("Duplicate directory model_id".into());
            }
            let mut scenarios = BTreeMap::new();
            let sidecar = path.with_extension("request.json");
            if sidecar.exists() {
                let meta = std::fs::symlink_metadata(&sidecar).map_err(|e| e.to_string())?;
                if !meta.is_file() || meta.len() > 1_000_000 {
                    return Err("Request sidecars must be regular files under 1 MB".into());
                }
                scenarios.insert(
                    "default".into(),
                    serde_json::from_slice(&std::fs::read(sidecar).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?,
                );
            }
            record.metadata["scenario_ids"] = json!(scenarios.keys().collect::<Vec<_>>());
            record.scenarios = scenarios;
            records.push(record);
        }
        if records.is_empty() {
            return Err("Directory contains no .qsp or model JSON files".into());
        }
        let version = hash(
            serde_json::to_string(&records.iter().map(|r| &r.metadata).collect::<Vec<_>>())
                .unwrap()
                .as_bytes(),
        );
        Ok(Self {
            version,
            records,
            availability_counts: Value::Null,
        })
    }
}

#[cfg(test)]
mod availability_integrity_tests {
    use super::*;
    #[test]
    fn declared_registry_digest_and_schema_fail_before_archive_loading() {
        let catalog = json!({"schema":"qspquest.models.library/v1","models":[],
            "availability_schema":crate::availability::SCHEMA,"availability_sha256":"a".repeat(64)});
        let raw = serde_json::to_vec(&catalog).unwrap();
        let registry = br#"{"schema":"qspquest.model-availability/v1","records":{}}"#;
        let error = Library::load(|name| match name {
            "catalog.json" => Ok(raw.clone()),
            "qspquest_models/availability.json" => Ok(registry.to_vec()),
            _ => panic!("archive loading must not begin"),
        })
        .err()
        .unwrap();
        assert!(error.contains("checksum differs"));
        let mut no_schema = catalog;
        no_schema
            .as_object_mut()
            .unwrap()
            .remove("availability_schema");
        let raw = serde_json::to_vec(&no_schema).unwrap();
        let error = Library::load(|name| {
            assert_eq!(name, "catalog.json");
            Ok(raw.clone())
        })
        .err()
        .unwrap();
        assert!(error.contains("requires its declared schema"));
    }
}
