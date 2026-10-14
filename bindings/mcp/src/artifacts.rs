//! Immutable, bounded result artifacts in an explicitly selected private directory.
use crate::{library::hash, local_fs::PrivateDirectory};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const URI_PREFIX: &str = "qspquest://artifacts/";
pub const CHUNK_BYTES: usize = 48_000;
pub const RETENTION_SECONDS: u64 = 86_400;
pub const EXPIRATION_MARKER_SECONDS: u64 = 604_800;
pub const MAX_BYTES: u64 = 128_000_000;
pub const MAX_TOTAL_BYTES: u64 = 1_000_000_000;
pub const MAX_LIVE_ARTIFACTS: usize = 256;
pub const MAX_RECORDS: usize = 512;
const MARKER: &str = ".qspquest-results-v1";
const MAGIC: &[u8] = b"qspquest.private-result-storage/v1\n";
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "ResultArtifactVersion")]
pub enum Version {
    #[serde(rename = "qspquest.result-artifact/v0.1")]
    V01,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: Version,
    pub uri: String,
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
    pub operation: String,
    pub result_schema: Option<String>,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub retention_seconds: u64,
    pub chunk_bytes: usize,
    pub chunk_count: usize,
    pub chunk_uri_prefix: String,
    pub instructions: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    descriptor: Descriptor,
    body_modified_ns: String,
}
#[derive(Debug)]
pub struct ArtifactError {
    pub code: &'static str,
    pub message: String,
}
fn failure(code: &'static str, message: impl Into<String>) -> ArtifactError {
    ArtifactError {
        code,
        message: message.into(),
    }
}
fn io_error(e: std::io::Error) -> ArtifactError {
    failure(
        if e.kind() == std::io::ErrorKind::NotFound {
            "artifact_not_found"
        } else {
            "artifact_storage"
        },
        e.to_string(),
    )
}
fn now() -> Result<u64, ArtifactError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failure("artifact_clock", "System clock precedes the Unix epoch"))?
        .as_secs())
}
fn identifier(id: &str) -> Result<(), ArtifactError> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(failure(
            "artifact_identifier",
            "Use the exact artifact URI returned by the calculation",
        ));
    }
    Ok(())
}
fn modified(file: &File) -> Result<String, ArtifactError> {
    Ok(file
        .metadata()
        .map_err(io_error)?
        .modified()
        .map_err(io_error)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            failure(
                "artifact_clock",
                "Artifact timestamp precedes the Unix epoch",
            )
        })?
        .as_nanos()
        .to_string())
}

pub struct Store {
    directory: PrivateDirectory,
    _lock: File,
    guard: Mutex<()>,
    total_limit: u64,
    entry_limit: u64,
    live_limit: usize,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, ArtifactError> {
        let directory = PrivateDirectory::open(path).map_err(io_error)?;
        let names = directory.names().map_err(io_error)?;
        if !names.iter().any(|n| n == MARKER) && !names.is_empty() {
            return Err(failure(
                "artifact_directory",
                "Use an empty private directory dedicated to result artifacts",
            ));
        }
        let (mut lock, new) = match directory.file(MARKER, true, true, true) {
            Ok(file) => (file, true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (
                directory
                    .file(MARKER, false, true, true)
                    .map_err(io_error)?,
                false,
            ),
            Err(e) => return Err(io_error(e)),
        };
        if new || lock.metadata().map_err(io_error)?.len() == 0 && names.iter().all(|n| n == MARKER)
        {
            lock.write_all(MAGIC).map_err(io_error)?;
            lock.sync_all().map_err(io_error)?;
            directory.sync().map_err(io_error)?;
        } else {
            let mut bytes = Vec::new();
            (&mut lock)
                .take(256)
                .read_to_end(&mut bytes)
                .map_err(io_error)?;
            if bytes != MAGIC {
                return Err(failure(
                    "artifact_directory",
                    "Result storage marker does not match this format",
                ));
            }
        }
        let store = Self {
            directory,
            _lock: lock,
            guard: Mutex::new(()),
            total_limit: MAX_TOTAL_BYTES,
            entry_limit: MAX_BYTES,
            live_limit: MAX_LIVE_ARTIFACTS,
        };
        store.cleanup(now()?)?;
        Ok(store)
    }
    fn receipt(&self, id: &str) -> Result<Receipt, ArtifactError> {
        identifier(id)?;
        let file = self
            .directory
            .file(&format!("{id}.receipt.json"), false, false, false)
            .map_err(io_error)?;
        if file.metadata().map_err(io_error)?.len() > 4096 {
            return Err(failure(
                "artifact_corrupt",
                "Artifact receipt exceeds its format bound",
            ));
        }
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes).map_err(io_error)?;
        let receipt: Receipt = serde_json::from_slice(&bytes)
            .map_err(|_| failure("artifact_corrupt", "Artifact receipt is invalid"))?;
        let d = &receipt.descriptor;
        if d.uri != format!("{URI_PREFIX}{id}")
            || identifier(&d.sha256).is_err()
            || d.bytes > MAX_BYTES
            || d.bytes == 0
            || d.media_type != "application/json"
            || d.retention_seconds != RETENTION_SECONDS
            || d.expires_at_unix_seconds
                != d.created_at_unix_seconds.saturating_add(RETENTION_SECONDS)
            || d.chunk_bytes != CHUNK_BYTES
            || d.chunk_count != (d.bytes as usize).div_ceil(CHUNK_BYTES)
            || d.chunk_uri_prefix != format!("{URI_PREFIX}{id}/chunks/")
            || d.operation.len() > 64
        {
            return Err(failure(
                "artifact_corrupt",
                "Artifact receipt fields are inconsistent",
            ));
        }
        Ok(receipt)
    }
    fn body(&self, id: &str, receipt: &Receipt) -> Result<File, ArtifactError> {
        let file = self
            .directory
            .file(&format!("{id}.json"), false, false, false)
            .map_err(io_error)?;
        if file.metadata().map_err(io_error)?.len() != receipt.descriptor.bytes
            || modified(&file)? != receipt.body_modified_ns
        {
            return Err(failure(
                "artifact_corrupt",
                "Artifact bytes or timestamp changed; do not interpret a partial result",
            ));
        }
        Ok(file)
    }
    // Called only during explicitly configured startup or artifact creation.
    fn cleanup(&self, time: u64) -> Result<(u64, usize, usize), ArtifactError> {
        let names = self.directory.names().map_err(io_error)?;
        if names.len() > 1 + MAX_RECORDS * 2 {
            return Err(failure(
                "artifact_budget",
                "Result storage has too many files; use a new dedicated directory",
            ));
        }
        for name in &names {
            if name == MARKER {
                continue;
            }
            let id = name
                .strip_suffix(".receipt.json")
                .or_else(|| name.strip_suffix(".json"))
                .ok_or_else(|| {
                    failure(
                        "artifact_directory",
                        "Unexpected file in dedicated result storage",
                    )
                })?;
            identifier(id)?;
            self.directory
                .file(name, false, false, false)
                .map_err(io_error)?;
        }
        let mut live = 0;
        for name in names.iter().filter(|n| n.ends_with(".receipt.json")) {
            let id = name.strip_suffix(".receipt.json").unwrap();
            let receipt = self.receipt(id)?;
            let d = &receipt.descriptor;
            if time >= d.expires_at_unix_seconds {
                match self.directory.remove(&format!("{id}.json")) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
                if time
                    >= d.expires_at_unix_seconds
                        .saturating_add(EXPIRATION_MARKER_SECONDS)
                {
                    self.directory.remove(name).map_err(io_error)?;
                }
            } else {
                self.body(id, &receipt)?;
                live += 1;
            }
        }
        // Interrupted writes publish no receipt. Remove only old owned orphan bodies.
        for name in names
            .iter()
            .filter(|n| n.ends_with(".json") && !n.ends_with(".receipt.json"))
        {
            let id = name.strip_suffix(".json").unwrap();
            if !names.contains(&format!("{id}.receipt.json")) {
                let file = self
                    .directory
                    .file(name, false, false, false)
                    .map_err(io_error)?;
                let timestamp = file
                    .metadata()
                    .map_err(io_error)?
                    .modified()
                    .map_err(io_error)?
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| failure("artifact_clock", "Orphan timestamp precedes Unix epoch"))?
                    .as_secs();
                drop(file);
                if time >= timestamp.saturating_add(RETENTION_SECONDS) {
                    self.directory.remove(name).map_err(io_error)?;
                }
            }
        }
        let mut total = 0u64;
        let mut records = 0;
        for name in self.directory.names().map_err(io_error)? {
            if name == MARKER {
                continue;
            }
            let file = self
                .directory
                .file(&name, false, false, false)
                .map_err(io_error)?;
            total = total
                .checked_add(file.metadata().map_err(io_error)?.len())
                .ok_or_else(|| failure("artifact_budget", "Artifact size sum overflow"))?;
            records += usize::from(name.ends_with(".receipt.json"));
        }
        Ok((total, live, records))
    }
    /// List live opaque references without changing retention or deleting files.
    pub fn uris(&self) -> Result<Vec<String>, ArtifactError> {
        let _guard = self
            .guard
            .lock()
            .map_err(|_| failure("artifact_storage", "Artifact storage lock poisoned"))?;
        let time = now()?;
        let mut uris = Vec::new();
        for name in self.directory.names().map_err(io_error)? {
            if let Some(id) = name.strip_suffix(".receipt.json") {
                let receipt = self.receipt(id)?;
                if time < receipt.descriptor.expires_at_unix_seconds {
                    self.body(id, &receipt)?;
                    uris.push(receipt.descriptor.uri);
                }
            }
        }
        uris.sort();
        Ok(uris)
    }
    pub fn put(&self, operation: &str, value: &Value) -> Result<Descriptor, ArtifactError> {
        self.put_at(operation, value, now()?)
    }
    fn put_at(
        &self,
        operation: &str,
        value: &Value,
        time: u64,
    ) -> Result<Descriptor, ArtifactError> {
        let _guard = self
            .guard
            .lock()
            .map_err(|_| failure("artifact_storage", "Result storage lock failed"))?;
        if operation.is_empty() || operation.len() > 64 || !value.is_object() {
            return Err(failure(
                "artifact_contract",
                "Artifact requires an operation and complete result object",
            ));
        }
        struct BoundedJson {
            bytes: Vec<u8>,
            limit: u64,
            exceeded: bool,
        }
        impl Write for BoundedJson {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if (self.bytes.len() as u64).saturating_add(data.len() as u64) > self.limit {
                    self.exceeded = true;
                    return Err(std::io::Error::other("Artifact byte budget exhausted"));
                }
                self.bytes.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut encoded = BoundedJson {
            bytes: Vec::new(),
            limit: self.entry_limit,
            exceeded: false,
        };
        if serde_json::to_writer(&mut encoded, value).is_err() {
            return Err(if encoded.exceeded {
                failure("artifact_budget", "Complete result exceeds the 128 MB artifact limit; reduce the grid or selected outputs")
            } else {
                failure("artifact_contract", "Result cannot be encoded as JSON")
            });
        }
        let bytes = encoded.bytes;
        let (used, live, records) = self.cleanup(time)?;
        if live >= self.live_limit
            || records >= MAX_RECORDS
            || used.saturating_add(bytes.len() as u64).saturating_add(4096) > self.total_limit
        {
            return Err(failure("artifact_budget","Result storage is full. Existing live artifacts were preserved; wait for retention cleanup or select a new dedicated directory"));
        }
        let digest = hash(&bytes);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| failure("artifact_clock", "Invalid system clock"))?
            .as_nanos();
        let id = hash(
            format!(
                "{digest}:{nonce}:{}:{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )
            .as_bytes(),
        );
        let descriptor=Descriptor{schema:Version::V01,uri:format!("{URI_PREFIX}{id}"),sha256:digest,bytes:bytes.len() as u64,media_type:"application/json".into(),operation:operation.into(),result_schema:value["contract"]["schema"].as_str().or_else(||value["schema"].as_str()).or_else(||value["effective_initial_values"]["schema"].as_str()).map(str::to_owned),created_at_unix_seconds:time,expires_at_unix_seconds:time.saturating_add(RETENTION_SECONDS),retention_seconds:RETENTION_SECONDS,chunk_bytes:CHUNK_BYTES,chunk_count:bytes.len().div_ceil(CHUNK_BYTES),chunk_uri_prefix:format!("{URI_PREFIX}{id}/chunks/"),instructions:"Read chunk_uri_prefix + each zero-based index below chunk_count. Concatenate the returned UTF-8 text without separators; verify bytes and SHA-256 before parsing the complete JSON. Retention starts at creation and survives server restart; a checksum verifies bytes, not producer identity or scientific qualification.".into()};
        let body_name = format!("{id}.json");
        let receipt_name = format!("{id}.receipt.json");
        let mut created = Vec::new();
        let write = (|| -> Result<(), ArtifactError> {
            let mut body = self
                .directory
                .file(&body_name, true, true, false)
                .map_err(io_error)?;
            created.push(body_name.clone());
            body.write_all(&bytes).map_err(io_error)?;
            body.sync_all().map_err(io_error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                body.set_permissions(std::fs::Permissions::from_mode(0o400))
                    .map_err(io_error)?;
            }
            body.sync_all().map_err(io_error)?;
            drop(body);
            // Close the writer before reading its final timestamp, including on Windows.
            let body = self
                .directory
                .file(&body_name, false, false, false)
                .map_err(io_error)?;
            let receipt = Receipt {
                descriptor: descriptor.clone(),
                body_modified_ns: modified(&body)?,
            };
            drop(body);
            let encoded = serde_json::to_vec(&receipt)
                .map_err(|_| failure("artifact_contract", "Cannot encode artifact receipt"))?;
            if encoded.len() > 4096 {
                return Err(failure(
                    "artifact_contract",
                    "Artifact receipt exceeds its format bound",
                ));
            }
            let mut file = self
                .directory
                .file(&receipt_name, true, true, false)
                .map_err(io_error)?;
            created.push(receipt_name.clone());
            file.write_all(&encoded).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o400))
                    .map_err(io_error)?;
            }
            file.sync_all().map_err(io_error)?;
            self.directory.sync().map_err(io_error)?;
            Ok(())
        })();
        if let Err(error) = write {
            for name in created.iter().rev() {
                let _ = self.directory.remove(name);
            }
            return Err(error);
        }
        Ok(descriptor)
    }
    pub fn descriptor(&self, id: &str) -> Result<Descriptor, ArtifactError> {
        self.descriptor_at(id, now()?)
    }
    fn descriptor_at(&self, id: &str, time: u64) -> Result<Descriptor, ArtifactError> {
        let _guard = self
            .guard
            .lock()
            .map_err(|_| failure("artifact_storage", "Result storage lock failed"))?;
        let receipt = self.receipt(id)?;
        if time >= receipt.descriptor.expires_at_unix_seconds {
            return Err(failure("artifact_expired","Artifact retention expired. Rerun the calculation; expiration markers remain for seven additional days"));
        }
        let mut file = self.body(id, &receipt)?;
        // Verify the complete persisted body once when the manifest is read.
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        if format!("{:x}", digest.finalize()) != receipt.descriptor.sha256 {
            return Err(failure(
                "artifact_corrupt",
                "Persisted result checksum differs from its receipt",
            ));
        }
        Ok(receipt.descriptor)
    }
    pub fn chunk(&self, id: &str, index: usize) -> Result<String, ArtifactError> {
        self.chunk_at(id, index, now()?)
    }
    fn chunk_at(&self, id: &str, index: usize, time: u64) -> Result<String, ArtifactError> {
        let _guard = self
            .guard
            .lock()
            .map_err(|_| failure("artifact_storage", "Result storage lock failed"))?;
        let receipt = self.receipt(id)?;
        let d = &receipt.descriptor;
        if time >= d.expires_at_unix_seconds {
            return Err(failure(
                "artifact_expired",
                "Artifact retention expired; rerun the calculation",
            ));
        }
        if index >= d.chunk_count {
            return Err(failure(
                "artifact_chunk",
                "Chunk index is outside this artifact",
            ));
        }
        let mut file = self.body(id, &receipt)?;
        let start = index * CHUNK_BYTES;
        let length = ((d.bytes as usize - start).min(CHUNK_BYTES + 3)) as u64;
        file.seek(SeekFrom::Start(start as u64)).map_err(io_error)?;
        let mut bytes = Vec::new();
        file.take(length)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        let mut begin = 0;
        while begin < bytes.len() && bytes[begin] & 0xc0 == 0x80 {
            begin += 1;
        }
        let mut end = CHUNK_BYTES.min(bytes.len());
        while end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
            end += 1;
        }
        String::from_utf8(bytes[begin..end].to_vec())
            .map_err(|_| failure("artifact_corrupt", "Artifact chunk is not valid UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "qspquest-artifact-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn id(d: &Descriptor) -> &str {
        d.uri.strip_prefix(URI_PREFIX).unwrap()
    }
    #[test]
    fn concurrent_quota_admission_preserves_published_results() {
        let scratch = Scratch::new("concurrent");
        let mut store = Store::open(&scratch.0).unwrap();
        store.live_limit = 2;
        let store = std::sync::Arc::new(store);
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store.put("simulate", &json!({"index":index,"values":[1,2,3]}))
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let retained: Vec<_> = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect();
        assert_eq!(retained.len(), 2);
        assert_ne!(retained[0].uri, retained[1].uri);
        for result in &results {
            if let Err(error) = result {
                assert_eq!(error.code, "artifact_budget");
            }
        }
        for descriptor in retained {
            assert_eq!(
                store.descriptor(id(descriptor)).unwrap().sha256,
                descriptor.sha256
            );
        }
        assert_eq!(store.uris().unwrap().len(), 2);
    }
    #[test]
    fn complete_large_utf8_artifact_survives_restart_and_verifies_every_byte() {
        let scratch = Scratch::new("large");
        let store = Store::open(&scratch.0).unwrap();
        let value = json!({"schema":"qspquest.test/v1","rows":"β🙂\n".repeat(600_000),"sides":["pre","post"],"identity":{"scope":"synthetic fixture"}});
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() > 4_000_000);
        let descriptor = store.put("simulate", &value).unwrap();
        assert!(Store::open(&scratch.0).is_err());
        drop(store);
        let reopened = Store::open(&scratch.0).unwrap();
        let loaded = reopened.descriptor(id(&descriptor)).unwrap();
        assert_eq!(loaded.sha256, descriptor.sha256);
        assert_eq!(
            loaded.expires_at_unix_seconds,
            descriptor.expires_at_unix_seconds
        );
        let mut all = String::new();
        for index in 0..loaded.chunk_count {
            let chunk = reopened.chunk(id(&descriptor), index).unwrap();
            assert!(chunk.len() <= CHUNK_BYTES + 3);
            all.push_str(&chunk);
        }
        assert_eq!(all.as_bytes(), bytes);
        assert_eq!(hash(all.as_bytes()), loaded.sha256);
        assert_eq!(serde_json::from_str::<Value>(&all).unwrap(), value);
        assert_eq!(
            reopened
                .chunk(id(&descriptor), loaded.chunk_count)
                .unwrap_err()
                .code,
            "artifact_chunk"
        );
    }
    #[test]
    fn expiry_is_specific_cleanup_retains_markers_and_quota_never_evicts_live_data() {
        let scratch = Scratch::new("expiry");
        let mut store = Store::open(&scratch.0).unwrap();
        store.live_limit = 1;
        let time = now().unwrap();
        let first = store
            .put_at("simulate", &json!({"rows":[1,2]}), time)
            .unwrap();
        assert_eq!(
            store
                .put_at("simulate", &json!({"rows":[3]}), time)
                .unwrap_err()
                .code,
            "artifact_budget"
        );
        assert_eq!(
            store.descriptor_at(id(&first), time).unwrap().sha256,
            first.sha256
        );
        let expiry = first.expires_at_unix_seconds;
        assert_eq!(
            store.descriptor_at(id(&first), expiry).unwrap_err().code,
            "artifact_expired"
        );
        store.cleanup(expiry).unwrap();
        assert!(!scratch.0.join(format!("{}.json", id(&first))).exists());
        assert!(scratch
            .0
            .join(format!("{}.receipt.json", id(&first)))
            .exists());
        assert_eq!(
            store.descriptor_at(id(&first), expiry).unwrap_err().code,
            "artifact_expired"
        );
        store.cleanup(expiry + EXPIRATION_MARKER_SECONDS).unwrap();
        assert_eq!(
            store
                .descriptor_at(id(&first), expiry + EXPIRATION_MARKER_SECONDS)
                .unwrap_err()
                .code,
            "artifact_not_found"
        );
        assert_eq!(
            store.descriptor("../elsewhere").unwrap_err().code,
            "artifact_identifier"
        );
    }
    #[test]
    fn corrupt_receipt_and_changed_payload_are_refused() {
        let scratch = Scratch::new("corrupt");
        let store = Store::open(&scratch.0).unwrap();
        let d = store.put("simulate", &json!({"rows":[1,2]})).unwrap();
        let body = scratch.0.join(format!("{}.json", id(&d)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&body, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::write(&body, b"{\"rows\":[9,8]}").unwrap();
        assert_eq!(
            store.descriptor(id(&d)).unwrap_err().code,
            "artifact_corrupt"
        );
        let receipt_path = scratch.0.join(format!("{}.receipt.json", id(&d)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&receipt_path, std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let mut receipt: Receipt =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        receipt.body_modified_ns = modified(&File::open(&body).unwrap()).unwrap();
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert_eq!(
            store.descriptor(id(&d)).unwrap_err().code,
            "artifact_corrupt"
        );
        std::fs::write(&receipt_path, b"{}").unwrap();
        assert_eq!(
            store.descriptor(id(&d)).unwrap_err().code,
            "artifact_corrupt"
        );
    }
    #[test]
    fn dedicated_directory_and_total_byte_limits_refuse_unrelated_or_partial_writes() {
        let scratch = Scratch::new("dedicated");
        std::fs::create_dir(&scratch.0).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(scratch.0.join("keep.txt"), "unrelated").unwrap();
        assert_eq!(
            Store::open(&scratch.0).err().unwrap().code,
            "artifact_directory"
        );
        assert_eq!(
            std::fs::read(scratch.0.join("keep.txt")).unwrap(),
            b"unrelated"
        );
        std::fs::remove_file(scratch.0.join("keep.txt")).unwrap();
        let mut store = Store::open(&scratch.0).unwrap();
        store.total_limit = 100;
        assert_eq!(
            store
                .put("simulate", &json!({"rows":[1]}))
                .unwrap_err()
                .code,
            "artifact_budget"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
        store.total_limit = MAX_TOTAL_BYTES;
        store.entry_limit = 1;
        assert_eq!(
            store
                .put("simulate", &json!({"rows":[1]}))
                .unwrap_err()
                .code,
            "artifact_budget"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn links_ownership_and_root_replacement_never_redirect_artifacts() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let scratch = Scratch::new("links");
        let store = Store::open(&scratch.0).unwrap();
        let d = store.put("simulate", &json!({"rows":[1]})).unwrap();
        let body = scratch.0.join(format!("{}.json", id(&d)));
        std::fs::remove_file(&body).unwrap();
        let target = scratch.0.join("outside.txt");
        std::fs::write(&target, "outside").unwrap();
        symlink(&target, &body).unwrap();
        assert!(store.descriptor(id(&d)).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"outside");
        std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(store.put("simulate", &json!({"rows":[2]})).is_err());
        std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let moved = scratch.0.with_extension("moved");
        std::fs::rename(&scratch.0, &moved).unwrap();
        std::fs::create_dir(&scratch.0).unwrap();
        std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(store.put("simulate", &json!({"rows":[3]})).is_err());
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 0);
        drop(store);
        std::fs::remove_dir_all(moved).unwrap();
    }
}
