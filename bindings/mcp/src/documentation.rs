//! Offline documentation metadata and canonical resource navigation.
use include_dir::{include_dir, Dir};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const INDEX: &str = include_str!("../assets/documentation.json");
static GUIDES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/guides");

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentationIndex {
    pub schema: String,
    pub package_version: String,
    pub scope: String,
    pub capabilities_uri: String,
    pub capability_catalog_uri: String,
    pub topics: std::collections::BTreeMap<String, String>,
    pub examples: Vec<Example>,
    pub resources: Vec<Resource>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub id: String,
    pub model_uri: String,
    pub request_uri: String,
    pub operation: String,
    pub scope: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub uri: String,
    pub title: String,
    pub mime_type: String,
    pub asset: String,
    pub source: String,
    pub source_sha256: String,
    pub content_sha256: String,
    pub anchors: Vec<String>,
}

pub fn index() -> DocumentationIndex {
    serde_json::from_str(INDEX).expect("Generated documentation index must match its contract")
}

pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(DocumentationIndex)).unwrap()
}

pub fn resources() -> Vec<(String, String)> {
    let mut resources = vec![
        ("qspquest://documentation".into(), "application/json".into()),
        (
            "qspquest://schema/documentation".into(),
            "application/json".into(),
        ),
    ];
    resources.extend(index().resources.into_iter().map(|r| (r.uri, r.mime_type)));
    resources
}

pub fn read(uri: &str) -> Option<Result<(String, String), String>> {
    if uri == "qspquest://documentation" {
        return Some(Ok(("application/json".into(), INDEX.into())));
    }
    if uri == "qspquest://schema/documentation" {
        return Some(Ok(("application/json".into(), schema().to_string())));
    }
    let (base, fragment) = uri
        .split_once('#')
        .map_or((uri, None), |(a, b)| (a, Some(b)));
    let resource = index().resources.into_iter().find(|r| r.uri == base)?;
    if fragment.is_some_and(|anchor| !resource.anchors.iter().any(|a| a == anchor)) {
        return Some(Err(
            "Documentation heading is not present in this version".into()
        ));
    }
    let text = if let Some(path) = resource.asset.strip_prefix("guides/") {
        GUIDES.get_file(path).and_then(|file| file.contents_utf8())
    } else {
        match resource.asset.as_str() {
            "capability-catalog.schema.json" => {
                Some(include_str!("../assets/capability-catalog.schema.json"))
            }
            "model.schema.json" => Some(include_str!("../assets/model.schema.json")),
            "expression.schema.json" => Some(include_str!("../assets/expression.schema.json")),
            "regimen.schema.json" => Some(include_str!("../assets/regimen.schema.json")),
            "run.schema.json" => Some(include_str!("../assets/run.schema.json")),
            "result.schema.json" => Some(include_str!("../assets/result.schema.json")),
            "initial-values-request.schema.json" => {
                Some(include_str!("../assets/initial-values-request.schema.json"))
            }
            "initial-values-plan.schema.json" => {
                Some(include_str!("../assets/initial-values-plan.schema.json"))
            }
            "initial-values-result.schema.json" => {
                Some(include_str!("../assets/initial-values-result.schema.json"))
            }
            "run-plan.schema.json" => Some(include_str!("../assets/run-plan.schema.json")),
            "scan-plan.schema.json" => Some(include_str!("../assets/scan-plan.schema.json")),
            "morris-plan.schema.json" => Some(include_str!("../assets/morris-plan.schema.json")),
            "periodic-plan.schema.json" => {
                Some(include_str!("../assets/periodic-plan.schema.json"))
            }
            "scalar-fit-plan.schema.json" => {
                Some(include_str!("../assets/scalar-fit-plan.schema.json"))
            }
            "fit-plan.schema.json" => Some(include_str!("../assets/fit-plan.schema.json")),
            "sensitivity-plan.schema.json" => {
                Some(include_str!("../assets/sensitivity-plan.schema.json"))
            }
            "gaussian-objective-plan.schema.json" => Some(include_str!(
                "../assets/gaussian-objective-plan.schema.json"
            )),
            "fit-request.schema.json" => Some(include_str!("../assets/fit-request.schema.json")),
            "fit-result.schema.json" => Some(include_str!("../assets/fit-result.schema.json")),
            "scalar-fit-request.schema.json" => {
                Some(include_str!("../assets/scalar-fit-request.schema.json"))
            }
            "scalar-fit-result.schema.json" => {
                Some(include_str!("../assets/scalar-fit-result.schema.json"))
            }
            "sensitivity-request.schema.json" => {
                Some(include_str!("../assets/sensitivity-request.schema.json"))
            }
            "sensitivity-result.schema.json" => {
                Some(include_str!("../assets/sensitivity-result.schema.json"))
            }
            "scan-request.schema.json" => Some(include_str!("../assets/scan-request.schema.json")),
            "scan-result.schema.json" => Some(include_str!("../assets/scan-result.schema.json")),
            "morris-request.schema.json" => {
                Some(include_str!("../assets/morris-request.schema.json"))
            }
            "morris-result.schema.json" => {
                Some(include_str!("../assets/morris-result.schema.json"))
            }
            "steady-state-request.schema.json" => {
                Some(include_str!("../assets/steady-state-request.schema.json"))
            }
            "steady-state-result.schema.json" => {
                Some(include_str!("../assets/steady-state-result.schema.json"))
            }
            "cycle-iteration-request.schema.json" => Some(include_str!(
                "../assets/cycle-iteration-request.schema.json"
            )),
            "cycle-iteration-result.schema.json" => {
                Some(include_str!("../assets/cycle-iteration-result.schema.json"))
            }
            _ => None,
        }
    };
    Some(
        text.map(|text| (resource.mime_type, text.to_owned()))
            .ok_or_else(|| "Documentation asset is absent".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn every_advertised_documentation_asset_is_readable_with_its_exact_digest() {
        for resource in index().resources {
            let (mime, content) = read(&resource.uri)
                .unwrap()
                .unwrap_or_else(|error| panic!("{}: {error}", resource.uri));
            assert_eq!(mime, resource.mime_type);
            assert_eq!(
                format!("{:x}", Sha256::digest(content.as_bytes())),
                resource.content_sha256,
                "{}",
                resource.uri
            );
        }
    }
    #[test]
    fn unsupported_namespaces_are_not_documentation_resources() {
        for uri in [
            "other://guide/requests",
            "https://example.invalid",
            "qspquest://missing",
        ] {
            assert!(read(uri).is_none());
        }
    }
}
