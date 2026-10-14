//! Content-bound, bounded inspection of complete native declarations.
use crate::{contract::JsonValueSchema, core::usability, library::Record, policy};
use qspquest_core::model as native;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_LIMIT: usize = 32;
pub const MAX_LIMIT: usize = 128;
const PAGE_BUDGET: usize = 40_000;
fn default_limit() -> usize {
    DEFAULT_LIMIT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Scalars,
    States,
    Outputs,
    Equations,
}
impl Section {
    pub const ALL: [Self; 4] = [Self::Scalars, Self::States, Self::Outputs, Self::Equations];
    pub fn name(self) -> &'static str {
        match self {
            Self::Scalars => "scalars",
            Self::States => "states",
            Self::Outputs => "outputs",
            Self::Equations => "equations",
        }
    }
    fn parse(s: &str) -> Result<Self, Value> {
        Self::ALL
            .into_iter()
            .find(|v| v.name() == s)
            .ok_or_else(|| {
                policy::error(
                    "inspection_section",
                    "Use scalars, states, outputs or equations from the inspection index.",
                )
            })
    }
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub section: Section,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Bind later pages to the exact native document, aliases and archive in the index.
    pub expected_inspection_hash: Option<String>,
}
/// Exact names and advertised resources for one content-bound model.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub scenarios: Vec<String>,
    /// Complete inspection index, page or record URIs for this model and hash.
    #[serde(default)]
    pub resources: Vec<String>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedScenario {
    pub scenario_id: String,
    pub request_sha256: String,
    /// The complete stored request, with native names, units and observation sides.
    #[schemars(with = "JsonValueSchema")]
    pub request: Value,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordDocument {
    pub context: Context,
    pub section: Section,
    pub entry: Entry,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ResourceData {
    Index(Index),
    Page(Page),
    Record(RecordDocument),
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedResource {
    pub uri: String,
    pub value: ResourceData,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub context: Context,
    pub execution_available: bool,
    /// One mapping per requested name, preserving its request order.
    pub output_mapping: Vec<crate::contract::OutputMapping>,
    /// Complete declarations, deduplicated in first-requested canonical order.
    pub outputs: Vec<Entry>,
    pub scenarios: Vec<SelectedScenario>,
    pub resources: Vec<SelectedResource>,
    pub index_resource: String,
    pub notice: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub model_id: String,
    pub model_version: Option<String>,
    pub archive_sha256: Option<String>,
    /// Hash of the loaded archive or standalone source bytes, when available.
    pub record_sha256: Option<String>,
    pub native_document_sha256: Option<String>,
    pub metadata_sha256: String,
    pub inspection_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectionChange {
    pub expected_inspection_hash: String,
    pub current_inspection_hash: String,
    pub model_id: String,
    pub model_version: Option<String>,
    pub inspection_index_resource: String,
    pub migration: Migration,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Migration {
    InspectCurrentBindings,
}

pub fn check_context(r: &Record, expected: Option<&str>) -> Result<(), Value> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if expected.len() != 71
        || !expected.starts_with("sha256:")
        || !expected[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(policy::error(
            "inspection_hash",
            "Use the exact sha256: digest from inspection.context.inspection_hash.",
        ));
    }
    let c = context(r)?;
    if expected == c.inspection_hash {
        return Ok(());
    }
    let mut error = policy::error("inspection_changed", "Model, archive or aliases changed. Inspect the current bindings and explicitly migrate names and units before preparing another request.");
    error["error"]["inspection_change"] = json!(InspectionChange {
        expected_inspection_hash: expected.into(),
        current_inspection_hash: c.inspection_hash.clone(),
        model_id: c.model_id.clone(),
        model_version: c.model_version.clone(),
        inspection_index_resource: format!("{}index", base(&c)),
        migration: Migration::InspectCurrentBindings,
    });
    Err(error)
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SectionIndex {
    pub section: Section,
    pub record_count: usize,
    pub first_page_resource: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub schema: IndexVersion,
    pub context: Context,
    pub execution_available: bool,
    pub sections: Vec<SectionIndex>,
    pub default_limit: usize,
    pub maximum_limit: usize,
    pub index_resource: String,
    pub complete_details_resource: String,
    pub notice: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
pub enum IndexVersion {
    #[serde(rename = "qspquest.model-inspection/v0.1")]
    V01,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alias {
    pub name: String,
    pub declared_source_unit: Option<String>,
    pub request_unit: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputData {
    #[schemars(with = "JsonValueSchema")]
    pub native: native::Output,
    pub descriptor: crate::contract::OutputDescriptor,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativeRecord {
    Parameter(#[schemars(with = "JsonValueSchema")] native::Parameter),
    Covariate(#[schemars(with = "JsonValueSchema")] native::Covariate),
    Individual(#[schemars(with = "JsonValueSchema")] native::IndividualParameter),
    Definition(#[schemars(with = "JsonValueSchema")] native::Definition),
    State(#[schemars(with = "JsonValueSchema")] native::State),
    Output(OutputData),
    FixedSwitch(#[schemars(with = "JsonValueSchema")] native::FixedSwitch),
    DoseHistory(#[schemars(with = "JsonValueSchema")] native::DoseHistoryBinding),
    DoseSwitch(#[schemars(with = "JsonValueSchema")] native::DoseSwitch),
    LinearPk(#[schemars(with = "JsonValueSchema")] native::LinearPkDefinition),
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub ordinal: usize,
    /// Exact JSON pointer in the retained native document.
    pub native_path: String,
    pub canonical_name: Option<String>,
    pub request_aliases: Option<Vec<Alias>>,
    /// Null only when the complete record exceeds the inline page budget.
    pub record: Option<NativeRecord>,
    pub record_resource: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub context: Context,
    pub section: Section,
    pub offset: usize,
    pub requested_limit: usize,
    pub record_count: usize,
    pub returned_count: usize,
    pub entries: Vec<Entry>,
    pub next_offset: Option<usize>,
    pub next_page_resource: Option<String>,
    pub page_resource: String,
    pub index_resource: String,
    pub notice: String,
}
fn context(r: &Record) -> Result<Context, Value> {
    let digest = |v: &Value| {
        qspquest_core::identity::canonical_hash(v)
            .map_err(|e| policy::error("inspection_metadata", e.to_string()))
    };
    let model_id = r.metadata["id"]
        .as_str()
        .ok_or_else(|| policy::error("inspection_metadata", "Model identifier is missing."))?
        .to_owned();
    let model_version = r.metadata["version"].as_str().map(str::to_owned);
    let record_sha256 = (!r.archive_hash.is_empty()).then(|| r.archive_hash.clone());
    let archive_sha256 = r.metadata["omex_sha256"].as_str().map(str::to_owned);
    let native_document_sha256 = (!r.model.is_null()).then(|| digest(&r.model)).transpose()?;
    let named = usability::augment(&r.index["native_execution"], &r.model);
    let mut inspected_metadata = json!({"metadata":r.metadata,"index":r.index,"input_bindings":{"parameters":named["parameters"],"states":named["states"]}});
    inspected_metadata["scenario_requests_sha256"] = json!(digest(&json!(r.scenarios))?);
    if let Some(profiles) = &r.precision_profiles {
        inspected_metadata["precision_profiles"] = json!({"file_sha256":profiles.file_sha256,"manifest_sha256":digest(&json!(profiles.manifest))?});
    }
    let metadata_sha256 = digest(&inspected_metadata)?;
    let inspection_hash = digest(
        &json!({"model_id":model_id,"model_version":model_version,"archive_sha256":archive_sha256,"record_sha256":record_sha256,"native_document_sha256":native_document_sha256,"metadata_sha256":metadata_sha256}),
    )?;
    Ok(Context {
        model_id,
        model_version,
        archive_sha256,
        record_sha256,
        native_document_sha256,
        metadata_sha256,
        inspection_hash,
    })
}
fn base(c: &Context) -> String {
    format!(
        "qspquest://models/{}/inspection/{}/",
        c.model_id,
        c.inspection_hash.trim_start_matches("sha256:")
    )
}
fn page_uri(c: &Context, section: Section, offset: usize, limit: usize) -> String {
    format!("{}{}/{offset}/{limit}", base(c), section.name())
}
fn groups(section: Section) -> &'static [(&'static str, &'static str)] {
    match section {
        Section::Scalars => &[
            ("parameters", "parameter"),
            ("covariates", "covariate"),
            ("individual", "individual"),
            ("definitions", "definition"),
        ],
        Section::States => &[("states", "state"), ("linear_pk", "linear_pk")],
        Section::Outputs => &[("outputs", "output")],
        Section::Equations => &[
            ("states", "state"),
            ("individual", "individual"),
            ("definitions", "definition"),
            ("outputs", "output"),
            ("switches", "fixed_switch"),
            ("dose_history", "dose_history"),
            ("dose_switches", "dose_switch"),
            ("linear_pk", "linear_pk"),
        ],
    }
}
fn declarations(r: &Record, section: Section) -> Vec<(String, &Value, &'static str)> {
    let mut rows = Vec::new();
    for &(field, kind) in groups(section) {
        if field == "linear_pk" {
            if !r.model[field].is_null() {
                rows.push((format!("/{field}"), &r.model[field], kind));
            }
        } else if let Some(values) = r.model[field].as_array() {
            rows.extend(
                values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("/{field}/{i}"), v, kind)),
            );
        }
    }
    rows
}
pub fn index(r: &Record) -> Result<Index, Value> {
    let context = context(r)?;
    let sections = if r.model.is_null() {
        Vec::new()
    } else {
        Section::ALL
            .into_iter()
            .map(|section| SectionIndex {
                section,
                record_count: declarations(r, section).len(),
                first_page_resource: page_uri(&context, section, 0, DEFAULT_LIMIT),
            })
            .collect()
    };
    Ok(Index{schema:IndexVersion::V01,execution_available:!r.model.is_null(),sections,default_limit:DEFAULT_LIMIT,maximum_limit:MAX_LIMIT,index_resource:format!("{}index",base(&context)),complete_details_resource:format!("qspquest://models/{}/details/{}",context.model_id,context.model_version.as_deref().unwrap_or("")),context,
        notice:"Records follow native declaration order within the listed section groups. Scalars include parameters, covariates and derived bindings. Equations include full state initial/RHS/dosing declarations, derived bindings, outputs, switches, dose history and linear PK declarations. A linear PK record contains its declared compartment states rather than invented expanded equations. Native units and archive source labels remain distinct; inspection does not qualify execution or science. Full details retain requirements, invariants and other metadata.".into()})
}
fn aliases(native: &Value, table: &str, name: &str) -> Vec<Alias> {
    native[table]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(alias, v)| {
            alias.as_str() != name && (v["native_name"] == name || v.as_str() == Some(name))
        })
        .map(|(alias, v)| Alias {
            name: alias.clone(),
            declared_source_unit: v["declared_source_unit"]
                .as_str()
                .or(v["source_unit"].as_str())
                .or(v["unit"].as_str())
                .map(str::to_owned),
            request_unit: v["request_unit"].as_str().map(str::to_owned),
        })
        .collect()
}
fn entry(
    r: &Record,
    c: &Context,
    section: Section,
    ordinal: usize,
    named: &Value,
) -> Result<Entry, Value> {
    let rows = declarations(r, section);
    let (path, value, kind) = rows.get(ordinal).ok_or_else(|| {
        policy::error(
            "inspection_offset",
            "Record offset is outside the section; read its index or page count.",
        )
    })?;
    let record: NativeRecord = if *kind == "output" {
        let descriptor = usability::output_descriptors(&json!({"outputs":[value]}), named)
            .into_iter()
            .find(|d| value["name"] == d.canonical_output)
            .ok_or_else(|| {
                policy::error(
                    "inspection_metadata",
                    "Native output descriptor is missing.",
                )
            })?;
        serde_json::from_value(json!({"kind":kind,"data":{"native":value,"descriptor":descriptor}}))
    } else {
        serde_json::from_value(json!({"kind":kind,"data":value}))
    }
    .map_err(|e| {
        policy::error(
            "inspection_metadata",
            format!("Invalid native inspection record at {path}: {e}"),
        )
    })?;
    let name = value["name"].as_str().map(str::to_owned);
    let table = match *kind {
        "parameter" => "parameters",
        "state" => "states",
        "output" => "outputs",
        _ => "",
    };
    let input_tables = if matches!(*kind, "parameter" | "state") {
        named
    } else {
        &r.index["native_execution"]
    };
    let mut request_aliases = name
        .as_deref()
        .map(|n| aliases(input_tables, table, n))
        .unwrap_or_default();
    if let NativeRecord::Output(output) = &record {
        request_aliases = output
            .descriptor
            .request_aliases
            .iter()
            .filter(|a| Some(&a.name) != name.as_ref())
            .map(|a| Alias {
                name: a.name.clone(),
                declared_source_unit: a.declared_source_unit.clone(),
                request_unit: None,
            })
            .collect();
    }
    Ok(Entry {
        ordinal,
        native_path: path.clone(),
        canonical_name: name,
        request_aliases: Some(request_aliases),
        record: Some(record),
        record_resource: format!("{}{}/items/{ordinal}", base(c), section.name()),
    })
}
pub fn page(r: &Record, request: &PageRequest) -> Result<Page, Value> {
    if request.limit == 0 || request.limit > MAX_LIMIT {
        return Err(policy::error(
            "inspection_limit",
            "Use a page limit from 1 to 128; the default is 32.",
        ));
    }
    let c = context(r)?;
    check_context(r, request.expected_inspection_hash.as_deref())?;
    if r.model.is_null() {
        return Err(policy::error("inspection_unavailable","This record has no native executable document. Inspect its model card and execution blocker; no substitute model is implied."));
    }
    let count = declarations(r, request.section).len();
    if request.offset > count {
        return Err(policy::error("inspection_offset","Page offset exceeds the section count. Read the inspection index and returned next_offset."));
    }
    let named = usability::augment(&r.index["native_execution"], &r.model);
    let mut entries = Vec::new();
    let mut bytes = 0;
    for ordinal in request.offset..count.min(request.offset.saturating_add(request.limit)) {
        let mut row = entry(r, &c, request.section, ordinal, &named)?;
        let size = serde_json::to_vec(&row).unwrap().len();
        if size > PAGE_BUDGET {
            row.record = None;
            if serde_json::to_vec(&row).unwrap().len() > PAGE_BUDGET {
                row.request_aliases = None;
            }
        }
        let size = serde_json::to_vec(&row).unwrap().len();
        if !entries.is_empty() && bytes + size > PAGE_BUDGET {
            break;
        }
        bytes += size;
        entries.push(row);
    }
    let next = request.offset + entries.len();
    let next_offset = (next < count).then_some(next);
    Ok(Page{page_resource:page_uri(&c,request.section,request.offset,request.limit),index_resource:format!("{}index",base(&c)),context:c.clone(),section:request.section,offset:request.offset,requested_limit:request.limit,record_count:count,returned_count:entries.len(),entries,next_offset,next_page_resource:next_offset.map(|n|page_uri(&c,request.section,n,request.limit)),notice:"A page may contain fewer records than its requested limit to preserve the response budget. Follow next_offset or next_page_resource, not offset plus requested_limit. A null record has its complete content at record_resource, which may use verified UTF-8 chunks. Source declarations do not establish current-engine or scientific qualification.".into()})
}
/// Resolve the complete batch before returning anything; no partial selection.
pub fn select(r: &Record, request: &SelectionRequest) -> Result<Selection, Value> {
    let count = request.outputs.len() + request.scenarios.len() + request.resources.len();
    if count == 0 || count > 32 {
        return Err(policy::error(
            "inspection_selection_limit",
            "Select 1 to 32 total output names, scenario IDs and inspection resources.",
        ));
    }
    for values in [&request.outputs, &request.scenarios, &request.resources] {
        let mut seen = std::collections::BTreeSet::new();
        for value in values {
            if value.is_empty() || value.len() > 1024 || !seen.insert(value) {
                return Err(policy::error("inspection_selection_names", "Use unique, nonempty exact names or URIs, each at most 1024 UTF-8 bytes, within each selection group."));
            }
        }
    }
    if r.model.is_null() {
        return Err(policy::error("inspection_unavailable", "This record has no native executable document. Inspect its discovery record; no substitute model is implied."));
    }
    let c = context(r)?;
    let named = usability::augment(&r.index["native_execution"], &r.model);
    let mapping = usability::output_mapping(&r.model, &named, &request.outputs)
        .map_err(|error| policy::error("inspection_output", error))?;
    let rows = declarations(r, Section::Outputs);
    let mut outputs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for selected in &mapping {
        if seen.insert(&selected.canonical_output) {
            let ordinal = rows
                .iter()
                .position(|(_, value, _)| value["name"] == selected.canonical_output)
                .ok_or_else(|| {
                    policy::error(
                        "inspection_output",
                        "The selected output has no native declaration.",
                    )
                })?;
            outputs.push(entry(r, &c, Section::Outputs, ordinal, &named)?);
        }
    }
    let mut scenarios = Vec::new();
    for id in &request.scenarios {
        let value = r.scenarios.get(id).ok_or_else(|| policy::error("inspection_scenario", format!("Scenario '{id}' is not stored for this exact model version; inspect its scenario_ids.")))?;
        if serde_json::to_vec(value).unwrap().len() > PAGE_BUDGET {
            return Err(policy::error("inspection_selection_budget", "Stored scenario exceeds the 40000-byte selection budget. Retrieve it separately with get_scenario_request; no partial selection was returned."));
        }
        let _: qspquest_core::run::RunRequest =
            serde_json::from_value(value.clone()).map_err(|error| {
                policy::error(
                    "inspection_scenario",
                    format!("Stored scenario '{id}' is not a complete native run request: {error}"),
                )
            })?;
        let request_sha256 = qspquest_core::identity::canonical_hash(value)
            .map_err(|error| policy::error("inspection_scenario", error.to_string()))?;
        scenarios.push(SelectedScenario {
            scenario_id: id.clone(),
            request_sha256,
            request: value.clone(),
        });
    }
    let mut resources = Vec::new();
    for uri in &request.resources {
        let prefix = format!("qspquest://models/{}/inspection/", c.model_id);
        let tail = uri.strip_prefix(&prefix).ok_or_else(|| policy::error("inspection_selection_resource", "Select exact advertised inspection URIs for this model and content hash; files, URLs, result resources and other models are not admitted."))?;
        let value = resource(r, tail)?;
        // Reject alternative spellings (leading zeros, trailing components, chunks).
        let advertised = value["index_resource"]
            .as_str()
            .filter(|_| value.get("schema").is_some())
            .or_else(|| value["page_resource"].as_str())
            .or_else(|| value["entry"]["record_resource"].as_str());
        if advertised != Some(uri.as_str()) {
            return Err(policy::error("inspection_selection_resource", "Use the exact advertised index, page or record URI, without chunk suffixes or alternate integer spellings."));
        }
        let data = if value.get("schema").is_some() {
            serde_json::from_value(value).map(ResourceData::Index)
        } else if value.get("page_resource").is_some() {
            serde_json::from_value(value).map(ResourceData::Page)
        } else {
            serde_json::from_value(value).map(ResourceData::Record)
        }
        .map_err(|error| policy::error("inspection_metadata", error.to_string()))?;
        resources.push(SelectedResource {
            uri: uri.clone(),
            value: data,
        });
    }
    let selected = Selection {
        index_resource: format!("{}index", base(&c)), context: c, execution_available: true,
        output_mapping: mapping, outputs, scenarios, resources,
        notice: "Exact selected declarations and stored requests only. No integration, approximation, unit conversion, scenario modification or scientific qualification. Output aliases retain their actual returned column and unit. All selections share the content-bound context. Inspection pages retain their own continuation or record-resource notices.".into(),
    };
    if serde_json::to_vec(&selected).unwrap().len() > PAGE_BUDGET {
        return Err(policy::error("inspection_selection_budget", "Complete selection exceeds the 40000-byte inline budget. Select fewer names or resources; use individual paged or chunked resources for a larger record. No partial selection was returned."));
    }
    Ok(selected)
}
/// Resolve only canonical advertised paths; source changes invalidate old resources.
pub fn resource(r: &Record, tail: &str) -> Result<Value, Value> {
    let c = context(r)?;
    let mut parts = tail.split('/');
    let digest = parts.next().unwrap_or("");
    if digest != c.inspection_hash.trim_start_matches("sha256:") {
        return Err(policy::error("inspection_changed","Inspection resource belongs to different content. Read the current model card for its index."));
    }
    let section = parts.next().unwrap_or("");
    if section == "index" && parts.next().is_none() {
        return Ok(json!(index(r)?));
    }
    let section = Section::parse(section)?;
    let a = parts.next().unwrap_or("");
    let b = parts.next();
    if parts.next().is_some() {
        return Err(policy::error(
            "inspection_resource",
            "Use an exact inspection resource URI from the index or page.",
        ));
    }
    let integer = |s: Option<&str>| {
        s.and_then(|v| v.parse::<usize>().ok()).ok_or_else(|| {
            policy::error(
                "inspection_resource",
                "Inspection offsets and limits must be nonnegative integers.",
            )
        })
    };
    if a == "items" {
        if r.model.is_null() {
            return Err(policy::error(
                "inspection_unavailable",
                "No native executable document is available.",
            ));
        }
        return Ok(
            json!({"context":c,"section":section,"entry":entry(r,&c,section,integer(b)?,&usability::augment(&r.index["native_execution"],&r.model))?}),
        );
    }
    Ok(json!(page(
        r,
        &PageRequest {
            section,
            offset: integer(Some(a))?,
            limit: integer(b)?,
            expected_inspection_hash: Some(c.inspection_hash)
        }
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Record {
        let source = "model synthetic_inspection
 time_unit = h
parameters
 cl = 0.8 [L/h] fixed
 v = 6 [L] fixed
 ka = 1.2 [1/h] fixed
states
 depot = 0 [mg]
 central = 0 [mg]
dynamics
 d/dt(depot) = -ka * depot
 d/dt(central) = ka * depot - cl / v * central
dosing
 depot [mg] = 1
outputs
 cp = central / v [mg/L]
end";
        Record {
            metadata: json!({"id":"synthetic_inspection","version":"1"}),
            model: crate::core::compute::execute(source, "validate_model", json!({})).unwrap()
                ["model"]
                .clone(),
            index: json!({"native_execution":{"parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}},"input_states":{"dose depot":{"native_name":"depot","source_unit":"mg","request_unit":"mg"}},"outputs":{"free concentration":{"native_name":"cp","unit":"mg/L","analyte":"drug","compartment":"central"}}}}),
            scenarios: Default::default(),
            archive_hash: String::new(),
            fitting: Value::Null,
            source: Some(source.into()),
            precision_profiles: None,
        }
    }
    #[test]
    fn exact_selection_preserves_alias_mapping_scenario_bytes_and_batched_resources() {
        let mut r = fixture();
        let stored = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000,"events":10,"output_values":100},"regimen":{"rtol":1e-8,"atol":1e-10,"start":{"value":0,"unit":"h"},"end":{"value":1,"unit":"h"},"samples":[],"administrations":[],"observations":[{"time":{"value":0,"unit":"h"},"side":"pre"},{"time":{"value":1,"unit":"h"},"side":"post"}]}});
        r.scenarios
            .insert("native_condition".into(), stored.clone());
        let indexed = index(&r).unwrap();
        let page = page(
            &r,
            &PageRequest {
                section: Section::Outputs,
                offset: 0,
                limit: 1,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        let resources = vec![
            indexed.index_resource.clone(),
            page.page_resource.clone(),
            page.entries[0].record_resource.clone(),
        ];
        let selected = select(
            &r,
            &SelectionRequest {
                outputs: vec!["free concentration".into(), "cp".into()],
                scenarios: vec!["native_condition".into()],
                resources: resources.clone(),
            },
        )
        .unwrap();
        assert_eq!(selected.output_mapping.len(), 2);
        assert_eq!(
            selected.output_mapping[0].requested_output,
            "free concentration"
        );
        assert_eq!(selected.output_mapping[0].canonical_output, "cp");
        assert_eq!(selected.output_mapping[0].returned_column, "cp");
        assert_eq!(selected.outputs.len(), 1);
        assert_eq!(selected.outputs[0].native_path, "/outputs/0");
        assert_eq!(selected.scenarios[0].request, stored);
        assert_eq!(
            selected.scenarios[0].request_sha256,
            qspquest_core::identity::canonical_hash(&stored).unwrap()
        );
        for (selected, uri) in selected.resources.iter().zip(resources) {
            let actual = serde_json::to_value(&selected.value).unwrap();
            assert_eq!(
                actual["data"],
                resource(&r, uri.split("/inspection/").nth(1).unwrap()).unwrap()
            );
        }
        let digest = indexed.context.inspection_hash;
        r.scenarios.get_mut("native_condition").unwrap()["regimen"]["rtol"] = json!(1e-5);
        assert_eq!(
            check_context(&r, Some(&digest)).err().unwrap()["error"]["code"],
            "inspection_changed"
        );
        r.scenarios.insert("native_condition".into(), stored);
        r.index["native_execution"]["outputs"]["free concentration"]["unit"] =
            json!("source_scale");
        assert_eq!(
            check_context(&r, Some(&digest)).err().unwrap()["error"]["code"],
            "inspection_changed"
        );
    }
    #[test]
    fn selection_refuses_missing_ambiguous_cross_context_and_oversized_batches() {
        let mut r = fixture();
        let make = |outputs: Vec<&str>, resources: Vec<String>| SelectionRequest {
            outputs: outputs.into_iter().map(str::to_owned).collect(),
            scenarios: vec![],
            resources,
        };
        assert_eq!(
            select(&r, &make(vec![], vec![])).err().unwrap()["error"]["code"],
            "inspection_selection_limit"
        );
        assert_eq!(
            select(&r, &make(vec!["cp", "cp"], vec![])).err().unwrap()["error"]["code"],
            "inspection_selection_names"
        );
        assert_eq!(
            select(&r, &make(vec!["cp", "missing"], vec![]))
                .err()
                .unwrap()["error"]["code"],
            "inspection_output"
        );
        let mut ambiguous = fixture();
        let mut extra = ambiguous.model["outputs"][0].clone();
        extra["name"] = json!("free concentration");
        ambiguous.model["outputs"]
            .as_array_mut()
            .unwrap()
            .push(extra);
        assert_eq!(
            select(&ambiguous, &make(vec!["free concentration"], vec![]))
                .err()
                .unwrap()["error"]["code"],
            "inspection_output"
        );
        assert_eq!(
            select(&r, &make(vec![], vec!["file:///tmp/private".into()]))
                .err()
                .unwrap()["error"]["code"],
            "inspection_selection_resource"
        );
        let page = page(
            &r,
            &PageRequest {
                section: Section::Outputs,
                offset: 0,
                limit: 1,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        assert_eq!(
            select(
                &r,
                &make(
                    vec![],
                    vec![page.page_resource.replace("/outputs/0/1", "/outputs/00/1")]
                )
            )
            .err()
            .unwrap()["error"]["code"],
            "inspection_selection_resource"
        );
        let old = page.entries[0].record_resource.clone();
        r.index["native_execution"]["outputs"]["free concentration"]["analyte"] = json!("changed");
        assert_eq!(
            select(&r, &make(vec![], vec![old])).err().unwrap()["error"]["code"],
            "inspection_changed"
        );
        r.index["native_execution"]["outputs"]["free concentration"]["analyte"] =
            json!("x".repeat(PAGE_BUDGET));
        assert_eq!(
            select(&r, &make(vec!["cp"], vec![])).err().unwrap()["error"]["code"],
            "inspection_selection_budget"
        );
        r.model = Value::Null;
        assert_eq!(
            select(&r, &make(vec!["cp"], vec![])).err().unwrap()["error"]["code"],
            "inspection_unavailable"
        );
    }
    #[test]
    fn complete_pages_preserve_unused_scalars_and_bind_aliases_before_continuation() {
        let mut r = fixture();
        for n in 0..150 {
            let mut p = r.model["parameters"][0].clone();
            p["name"] = json!(format!("unused_{n}"));
            r.model["parameters"].as_array_mut().unwrap().push(p);
        }
        let indexed = index(&r).unwrap();
        let digest = indexed.context.inspection_hash;
        let mut offset = 0;
        let mut names = Vec::new();
        loop {
            let p = page(
                &r,
                &PageRequest {
                    section: Section::Scalars,
                    offset,
                    limit: 16,
                    expected_inspection_hash: Some(digest.clone()),
                },
            )
            .unwrap();
            for e in &p.entries {
                let data = serde_json::to_value(e.record.as_ref().unwrap()).unwrap();
                assert_eq!(data["data"], *r.model.pointer(&e.native_path).unwrap());
                names.push(e.canonical_name.clone().unwrap());
            }
            match p.next_offset {
                Some(n) => {
                    assert!(n > offset);
                    offset = n
                }
                None => break,
            }
        }
        assert_eq!(names.len(), r.model["parameters"].as_array().unwrap().len());
        assert_eq!(names.last().unwrap(), "unused_149");
        r.index["native_execution"]["parameters"]["clearance"]["request_unit"] = json!("mL/h");
        let refused = page(
            &r,
            &PageRequest {
                section: Section::Scalars,
                offset: 0,
                limit: 16,
                expected_inspection_hash: Some(digest.clone()),
            },
        )
        .err()
        .unwrap();
        assert_eq!(refused["error"]["code"], "inspection_changed");
        assert_eq!(
            resource(
                &r,
                &format!("{}/scalars/0/16", digest.trim_start_matches("sha256:"))
            )
            .unwrap_err()["error"]["code"],
            "inspection_changed"
        );
        for (offset, limit, code) in [
            (0, 0, "inspection_limit"),
            (0, 129, "inspection_limit"),
            (999, 32, "inspection_offset"),
        ] {
            assert_eq!(
                page(
                    &r,
                    &PageRequest {
                        section: Section::Scalars,
                        offset,
                        limit,
                        expected_inspection_hash: None
                    }
                )
                .err()
                .unwrap()["error"]["code"],
                code
            );
        }
        r.model = Value::Null;
        assert!(!index(&r).unwrap().execution_available);
        assert_eq!(
            page(
                &r,
                &PageRequest {
                    section: Section::States,
                    offset: 0,
                    limit: 32,
                    expected_inspection_hash: None
                }
            )
            .err()
            .unwrap()["error"]["code"],
            "inspection_unavailable"
        );
    }
    #[test]
    fn equation_pages_preserve_initial_rhs_dosing_and_runtime_bindings() {
        let mut r = fixture();
        r.model["definitions"] =
            json!([{"name":"derived","expression":{"kind":"symbol","name":"cl"},"unit":"L/h"}]);
        r.model["individual"] =
            json!([{"name":"subject","expression":{"kind":"symbol","name":"cl"},"unit":"L/h"}]);
        r.model["switches"] = json!([{"name":"phase","time":{"kind":"literal","value":1.0}}]);
        r.model["dose_history"] = json!([{"name":"previous","target":"depot","quantity":"has_dose","simultaneous":"ordered"}]);
        // Use a serialized native enum value rather than inventing its spelling.
        r.model["dose_history"][0]["simultaneous"] =
            json!(qspquest_core::dose_history::SimultaneousDose::First);
        let p = page(
            &r,
            &PageRequest {
                section: Section::Equations,
                offset: 0,
                limit: 32,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        for e in p.entries {
            let data = serde_json::to_value(e.record.unwrap()).unwrap();
            let original = r.model.pointer(&e.native_path).unwrap();
            if data["kind"] != "output" {
                assert_eq!(&data["data"], original);
            }
            if data["kind"] == "output" {
                assert_eq!(&data["data"]["native"], original);
                assert_eq!(
                    data["data"]["descriptor"]["request_aliases"][1]["analyte"],
                    "drug"
                );
            }
            if data["kind"] == "state" {
                assert!(!data["data"]["initial"].is_null());
                assert!(!data["data"]["rhs"].is_null());
            }
        }
        assert_eq!(
            p.record_count,
            r.model["states"].as_array().unwrap().len() + 5
        );
    }
    #[test]
    fn linear_pk_declarations_keep_all_compartments_without_invented_rhs() {
        let mut r = fixture();
        r.model = crate::core::compute::execute(
            include_str!("../assets/templates/oral.qsp"),
            "validate_model",
            json!({}),
        )
        .unwrap()["model"]
            .clone();
        let p = page(
            &r,
            &PageRequest {
                section: Section::States,
                offset: 0,
                limit: 1,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        assert_eq!(p.record_count, 1);
        assert_eq!(p.entries[0].native_path, "/linear_pk");
        let data = serde_json::to_value(p.entries[0].record.as_ref().unwrap()).unwrap();
        assert_eq!(data["kind"], "linear_pk");
        assert_eq!(data["data"], r.model["linear_pk"]);
        assert_eq!(data["data"]["compartments"][0]["state"], "central");
        assert_eq!(data["data"]["depot"]["state"], "depot");
    }
    #[test]
    fn oversized_records_remain_complete_in_content_bound_resources() {
        let mut r = fixture();
        let rhs = r.model["states"][0]["rhs"].clone();
        r.model["states"][0]["rhs"] =
            json!({"kind":"call","name":"min","arguments":vec![rhs;1000]});
        let p = page(
            &r,
            &PageRequest {
                section: Section::States,
                offset: 0,
                limit: 32,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        assert!(p.entries[0].record.is_none());
        assert!(serde_json::to_vec(&p).unwrap().len() < PAGE_BUDGET + 4000);
        let tail = p.entries[0]
            .record_resource
            .split("/inspection/")
            .nth(1)
            .unwrap();
        let full = resource(&r, tail).unwrap();
        assert_eq!(full["entry"]["record"]["data"], r.model["states"][0]);
        let p = page(
            &r,
            &PageRequest {
                section: Section::States,
                offset: p.record_count,
                limit: 1,
                expected_inspection_hash: None,
            },
        )
        .unwrap();
        assert_eq!(p.returned_count, 0);
        assert!(p.next_offset.is_none());
        let schema = crate::contract::schema::<Page>();
        let variants = schema["$defs"]["NativeRecord"]["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 10);
        assert!(variants
            .iter()
            .all(|v| v["properties"]["data"]["$ref"].is_string()));
        assert_eq!(
            schema["$defs"]["InspectionDoc_State"]["additionalProperties"],
            false
        );
    }
}
