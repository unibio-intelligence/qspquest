//! Read-only, authenticated byte ranges for explicitly retained platform artifacts.
use crate::{
    jobs::{HttpJobBackend, ManagedArtifact, Scope},
    library::hash,
    policy::error,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub const URI_PREFIX: &str = "qspquest://managed-artifacts/";
pub const CHUNK_BYTES: u64 = 48_000;
pub const MAX_BYTES: u64 = 128_000_000;
pub const MAX_RETENTION_SECONDS: u64 = 604_800;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum RetentionVersion {
    #[serde(rename = "qspquest.platform-result-bytes/v1")]
    V1,
}
/// An upstream contract, not a declaration that an unqualified backend supports it.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub schema: RetentionVersion,
    #[schemars(extend("const" = true))]
    pub immutable: bool,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub retention_seconds: u64,
    pub result_schema: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum DescriptorVersion {
    #[serde(rename = "qspquest.managed-result-artifact/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: DescriptorVersion,
    pub uri: String,
    pub artifact_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
    pub result_schema: Option<String>,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub retention_seconds: u64,
    pub chunk_bytes: u64,
    pub chunk_count: u64,
    pub chunk_uri_prefix: String,
    pub instructions: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Retrieval {
    Available {
        descriptor: Descriptor,
    },
    Unavailable {
        code: String,
        reason: String,
    },
    Expired {
        code: String,
        expires_at_unix_seconds: u64,
    },
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub enum ChunkVersion {
    #[serde(rename = "qspquest.managed-artifact-chunk/v1")]
    V1,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Base64,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub schema: ChunkVersion,
    pub artifact_uri: String,
    pub artifact_sha256: String,
    pub index: u64,
    pub offset: u64,
    pub bytes: u64,
    pub sha256: String,
    pub encoding: Encoding,
    pub data: String,
}
pub fn clock() -> Result<u64, Value> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| {
            error(
                "artifact_clock",
                "Artifact retention requires a valid UTC clock",
            )
        })
}
pub fn retrieval(artifact: &ManagedArtifact, now: u64) -> Result<Retrieval, Value> {
    let Some(retention) = &artifact.result_retention else {
        return Ok(Retrieval::Unavailable { code: "managed_artifact_bytes_unavailable".into(),
            reason: "The platform descriptor has no versioned immutable-byte/retention contract. Metadata and its download link do not promise complete MCP retrieval.".into() });
    };
    if !retention.immutable
        || retention.retention_seconds == 0
        || retention.retention_seconds > MAX_RETENTION_SECONDS
        || retention
            .created_at_unix_seconds
            .checked_add(retention.retention_seconds)
            != Some(retention.expires_at_unix_seconds)
        || retention.created_at_unix_seconds > now
        || artifact.size_bytes > MAX_BYTES
        || artifact.media_type.is_empty()
        || artifact.media_type.len() > 200
        || artifact.media_type.chars().any(char::is_control)
        || retention
            .result_schema
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 200 || s.chars().any(char::is_control))
    {
        return Err(error(
            "platform_response",
            "Invalid immutable artifact size, schema or retention declaration",
        ));
    }
    if now >= retention.expires_at_unix_seconds {
        return Ok(Retrieval::Expired {
            code: "managed_artifact_expired".into(),
            expires_at_unix_seconds: retention.expires_at_unix_seconds,
        });
    }
    let uri = format!("{URI_PREFIX}{}/{}", artifact.id, artifact.checksum_hex);
    Ok(Retrieval::Available { descriptor: Descriptor {
        schema: DescriptorVersion::V1, uri: uri.clone(), artifact_id: artifact.id.clone(), organization_id: artifact.organization_id.clone(),
        project_id: artifact.project_id.clone(), sha256: artifact.checksum_hex.clone(), bytes: artifact.size_bytes,
        media_type: artifact.media_type.clone(), result_schema: retention.result_schema.clone(),
        created_at_unix_seconds: retention.created_at_unix_seconds, expires_at_unix_seconds: retention.expires_at_unix_seconds,
        retention_seconds: retention.retention_seconds, chunk_bytes: CHUNK_BYTES, chunk_count: artifact.size_bytes.div_ceil(CHUNK_BYTES),
        chunk_uri_prefix: format!("{uri}/chunks/"),
        instructions: "Use resources/read with the same verified session. Decode each base64 chunk, verify its index, byte range, chunk SHA-256 and artifact SHA-256; concatenate in order and verify the total bytes and full SHA-256 before interpreting or saving. Retrieval does not qualify the scientific result.".into(),
    }})
}
fn available(value: Retrieval) -> Result<Descriptor, Value> {
    match value {
        Retrieval::Available { descriptor } => Ok(descriptor),
        Retrieval::Unavailable { code, reason } => Err(error(&code, reason)),
        Retrieval::Expired { code, .. } => Err(error(
            &code,
            "The governed artifact retention has expired; its bytes are unavailable",
        )),
    }
}
/// The metadata-selected link is restricted to this exact configured artifact route.
/// No bearer token reaches another origin, object store, query or artifact ID.
pub fn download_url(base: &str, artifact: &ManagedArtifact) -> Result<reqwest::Url, Value> {
    let base = crate::deployment::endpoint(base)
        .map_err(|_| error("configuration", "Invalid platform artifact endpoint"))?;
    let expected = reqwest::Url::parse(&format!(
        "{}/artifacts/{}/download",
        base.as_str().trim_end_matches('/'),
        artifact.id
    ))
    .map_err(|_| error("configuration", "Invalid configured artifact route"))?;
    let href = &artifact.download_href;
    if href.contains(['\\', '?', '#', '%'])
        || href.chars().any(char::is_whitespace)
        || href.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(error(
            "managed_artifact_route",
            "Artifact byte retrieval requires the exact same-origin authenticated download route",
        ));
    }
    let actual = if href.starts_with('/') && !href.starts_with("//") {
        base.join(href)
    } else {
        reqwest::Url::parse(href)
    }
    .map_err(|_| error("managed_artifact_route", "Invalid artifact download route"))?;
    if actual != expected || !actual.username().is_empty() || actual.password().is_some() {
        return Err(error(
            "managed_artifact_route",
            "Artifact byte retrieval requires the exact same-origin authenticated download route",
        ));
    }
    Ok(actual)
}
pub fn parse_uri(uri: &str) -> Result<(&str, &str, Option<u64>), Value> {
    let tail = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| error("managed_artifact_uri", "Invalid managed artifact URI"))?;
    let parts: Vec<_> = tail.split('/').collect();
    if parts.len() != 2 && (parts.len() != 4 || parts[2] != "chunks") {
        return Err(error(
            "managed_artifact_uri",
            "Managed artifact URI requires an exact artifact ID and SHA-256",
        ));
    }
    crate::jobs::safe_id(parts[0])?;
    crate::jobs::preview_digest(parts[1]).map_err(|_| {
        error(
            "managed_artifact_uri",
            "Managed artifact URI requires its exact lowercase SHA-256",
        )
    })?;
    let chunk = if parts.len() == 4 {
        let index = parts[3]
            .parse::<u64>()
            .map_err(|_| error("managed_artifact_chunk", "Invalid artifact chunk index"))?;
        if parts[3] != index.to_string() {
            return Err(error(
                "managed_artifact_chunk",
                "Chunk index must use canonical decimal digits",
            ));
        }
        Some(index)
    } else {
        None
    };
    Ok((parts[0], parts[1], chunk))
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        output.push(TABLE[(a >> 2) as usize] as char);
        output.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        output.push(if part.len() > 1 {
            TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if part.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
impl HttpJobBackend {
    pub async fn artifact_resource(&self, scope: &Scope, uri: &str) -> Result<Value, Value> {
        scope.admitted()?;
        let _permit = crate::jobs::tenant_slots(&scope.tenant_id)
            .try_acquire_owned()
            .map_err(|_| {
                error(
                    "tenant_busy",
                    "This tenant already occupies its two compute/retrieval slots",
                )
            })?;
        let (id, expected_sha, index) = parse_uri(uri)?;
        let (artifact, _) = self.artifact_metadata(scope, id).await?;
        if artifact.checksum_hex != expected_sha {
            return Err(error(
                "managed_artifact_changed",
                "The governed artifact digest differs from the pinned resource URI",
            ));
        }
        let descriptor = available(retrieval(&artifact, clock()?)?)?;
        let url = download_url(&self.base, &artifact)?;
        let Some(index) = index else {
            return serde_json::to_value(descriptor)
                .map_err(|_| error("platform_response", "Cannot encode artifact descriptor"));
        };
        if index >= descriptor.chunk_count {
            return Err(error(
                "managed_artifact_chunk",
                "Chunk index exceeds the retained artifact",
            ));
        }
        let start = index * CHUNK_BYTES;
        let end = (start + CHUNK_BYTES).min(descriptor.bytes) - 1;
        let etag = format!("\"sha256-{expected_sha}\"");
        let mut response = self
            .client
            .get(url)
            .bearer_auth(&scope.bearer)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .header(reqwest::header::IF_MATCH, &etag)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| error("platform_unavailable", "Artifact byte request failed"))?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(error("managed_artifact_access_denied", "The platform denied this session or Project access; no artifact bytes were returned"));
        }
        if status.as_u16() == 410 {
            return Err(error(
                "managed_artifact_expired",
                "The governed artifact retention has expired",
            ));
        }
        if status.as_u16() == 412 {
            return Err(error(
                "managed_artifact_changed",
                "Artifact bytes differ from their pinned digest",
            ));
        }
        if status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(error("managed_artifact_range", "The platform must authorize the exact artifact and return HTTP 206 for a bounded immutable range; redirects and full-body fallbacks are refused"));
        }
        let expected_range = format!("bytes {start}-{end}/{}", descriptor.bytes);
        let headers = response.headers();
        if headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            != Some(expected_range.as_str())
            || headers
                .get(reqwest::header::ETAG)
                .and_then(|h| h.to_str().ok())
                != Some(etag.as_str())
            || headers
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|h| h.to_str().ok() != Some("identity"))
            || headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .map(|s| s.split(';').next().unwrap().trim())
                != descriptor.media_type.split(';').next().map(str::trim)
            || response
                .content_length()
                .is_some_and(|n| n != end - start + 1)
        {
            return Err(error(
                "managed_artifact_range",
                "Artifact range, content identity, media type or size differs",
            ));
        }
        let mut bytes = Vec::with_capacity((end - start + 1) as usize);
        while let Some(part) = response
            .chunk()
            .await
            .map_err(|_| error("platform_unavailable", "Artifact byte stream interrupted"))?
        {
            if bytes.len() as u64 + part.len() as u64 > end - start + 1 {
                return Err(error(
                    "managed_artifact_range",
                    "Artifact response exceeds its bounded range",
                ));
            }
            bytes.extend_from_slice(&part);
        }
        if bytes.len() as u64 != end - start + 1 {
            return Err(error(
                "managed_artifact_range",
                "Artifact range is truncated",
            ));
        }
        if clock()? >= descriptor.expires_at_unix_seconds {
            return Err(error(
                "managed_artifact_expired",
                "Artifact retention expired during retrieval",
            ));
        }
        serde_json::to_value(Chunk {
            schema: ChunkVersion::V1,
            artifact_uri: descriptor.uri,
            artifact_sha256: expected_sha.into(),
            index,
            offset: start,
            bytes: bytes.len() as u64,
            sha256: hash(&bytes),
            encoding: Encoding::Base64,
            data: base64(&bytes),
        })
        .map_err(|_| error("platform_response", "Cannot encode bounded artifact chunk"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact() -> ManagedArtifact {
        serde_json::from_value(json!({"id":"a","organization_id":"t","project_id":"p","run_id":null,"job_id":null,"kind":"result","name":"synthetic","media_type":"application/json","size_bytes":4_800_001,"checksum_hex":"a".repeat(64),"download_href":"/api/artifacts/a/download",
            "result_retention":{"schema":"qspquest.platform-result-bytes/v1","immutable":true,"created_at_unix_seconds":10,"expires_at_unix_seconds":110,"retention_seconds":100,"result_schema":"synthetic-only"}})).unwrap()
    }
    #[test]
    fn descriptors_bind_size_digest_retention_and_expiration() {
        let mut value = artifact();
        let descriptor = available(retrieval(&value, 11).unwrap()).unwrap();
        assert_eq!(descriptor.chunk_count, 101);
        assert!(descriptor.uri.ends_with(&"a".repeat(64)));
        assert!(matches!(
            retrieval(&value, 110).unwrap(),
            Retrieval::Expired { .. }
        ));
        value.result_retention = None;
        assert!(matches!(
            retrieval(&value, 11).unwrap(),
            Retrieval::Unavailable { .. }
        ));
        value = artifact();
        value.result_retention.as_mut().unwrap().immutable = false;
        assert!(retrieval(&value, 11).is_err());
        value = artifact();
        value
            .result_retention
            .as_mut()
            .unwrap()
            .expires_at_unix_seconds = 111;
        assert!(retrieval(&value, 11).is_err());
    }
    #[test]
    fn download_routes_never_forward_auth_to_another_origin_or_artifact() {
        let mut value = artifact();
        assert!(download_url("https://example.test/api", &value).is_ok());
        for href in [
            "https://other.test/api/artifacts/a/download",
            "//other.test/api/artifacts/a/download",
            "/api/artifacts/b/download",
            "/api/artifacts/a/download?token=x",
            "/api/artifacts/%61/download",
            "https://user:pass@example.test/api/artifacts/a/download",
        ] {
            value.download_href = href.into();
            assert!(download_url("https://example.test/api", &value).is_err());
        }
    }
    #[test]
    fn pinned_uri_and_binary_base64_are_exact() {
        let uri = format!("{URI_PREFIX}a/{}/chunks/0", "a".repeat(64));
        assert_eq!(parse_uri(&uri).unwrap().2, Some(0));
        assert!(parse_uri(&uri.replace("chunks/0", "chunks/00")).is_err());
        assert!(parse_uri(&uri.replace("/a/", "/../")).is_err());
        assert!(parse_uri(&uri.replace("chunks/0", "chunks/18446744073709551616")).is_err());
        for (input, encoded) in [
            (b"".as_slice(), ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (&[0, 255, 128], "AP+A"),
        ] {
            assert_eq!(base64(input), encoded);
        }
    }
}
