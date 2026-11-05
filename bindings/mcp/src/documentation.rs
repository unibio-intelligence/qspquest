//! Offline documentation metadata and canonical resource navigation.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const INDEX: &str = include_str!("../assets/documentation.json");
/// Relative links in embedded Markdown, rewritten to resource URIs when served:
/// `[byte_start, byte_end, original_target, replacement]` per resource.
const LINKS: &str = include_str!("../assets/documentation-links.json");
include!(concat!(env!("OUT_DIR"), "/documentation_sources.rs"));

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

/// The served text of one resource: its embedded source with links rewritten.
fn served_text(uri: &str) -> Option<String> {
    let (_, source) = SOURCES.iter().find(|(candidate, _)| *candidate == uri)?;
    let links: std::collections::BTreeMap<String, Vec<(usize, usize, String, String)>> =
        serde_json::from_str(LINKS).expect("Generated documentation links must be valid");
    let mut text = (*source).to_owned();
    for (start, end, original, replacement) in links.get(uri).into_iter().flatten().rev() {
        if text.get(*start..*end) != Some(original.as_str()) {
            return None;
        }
        text.replace_range(*start..*end, replacement);
    }
    Some(text)
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
    let text = served_text(&resource.uri);
    Some(
        text.map(|text| (resource.mime_type, text))
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
    fn embedded_sources_match_the_generated_index() {
        let resources = index().resources;
        assert_eq!(resources.len(), SOURCES.len());
        for resource in resources {
            let (_, source) = SOURCES
                .iter()
                .find(|(uri, _)| *uri == resource.uri)
                .unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(source.as_bytes())),
                resource.source_sha256,
                "{} is stale; run scripts/generate_mcp_resources.py",
                resource.source
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
