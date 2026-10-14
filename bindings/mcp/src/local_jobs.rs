//! Local, source-bound work journals and explicit offline job states.
use crate::{
    artifacts, contract::JsonValueSchema, library::hash, local_fs::PrivateDirectory, policy,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_UNITS: usize = 25;
pub const MAX_JOBS: usize = 64;
pub const MAX_ACTIVE: usize = 32;
pub const MAX_REVISIONS: usize = 128;
pub const RETENTION_SECONDS: u64 = 86_400;
pub const TOMBSTONE_SECONDS: u64 = 604_800;
const MAX_REQUEST_BYTES: usize = 2_000_000;
const MAX_STATE_BYTES: usize = 64_000;
const MAX_NAMES: usize = 1 + MAX_JOBS * (2 * MAX_REVISIONS + 1) + 512;
const MARKER: &str = ".qspquest-local-jobs-v1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Simulate,
    ScanParameters,
    FitScalar,
    FitIndividual,
    FitPopulation,
    SteadyState,
    MorrisScreening,
}
impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Simulate => "simulate",
            Self::ScanParameters => "scan_parameters",
            Self::FitScalar => "fit_scalar",
            Self::FitIndividual => "fit_individual",
            Self::FitPopulation => "fit_population",
            Self::SteadyState => "steady_state",
            Self::MorrisScreening => "morris_screening",
        }
    }
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub label: String,
    pub operation: Operation,
    #[schemars(with = "JsonValueSchema")]
    pub arguments: Value,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    /// A stable ASCII retry key within this local job directory; a new experiment needs a new key.
    pub idempotency_key: String,
    pub work: Vec<Unit>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobId {
    pub job_id: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resume {
    pub job_id: String,
    /// Explicitly allow replay of a unit dispatched before a process interruption but never committed.
    #[serde(default)]
    pub retry_uncommitted: bool,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct Payload {
    pub local_job: Descriptor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(rename = "LocalJobState")]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}
impl State {
    pub fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::CancelRequested)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LocalJobFailure")]
pub struct Failure {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LocalJobCompletedUnit")]
pub struct CompletedUnit {
    pub index: usize,
    pub label: String,
    pub operation: String,
    pub plan_hash: String,
    /// Scientific result contract status; job admission never supplies this status.
    pub scientific_status: String,
    pub artifact: Option<artifacts::Descriptor>,
    pub failure: Option<Failure>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LocalJobDescriptor")]
pub struct Descriptor {
    pub schema: String,
    pub job_id: String,
    pub state: State,
    pub request_sha256: String,
    pub prepared_request_sha256: String,
    pub executor_sha256: String,
    pub work_units: usize,
    pub completed: Vec<CompletedUnit>,
    /// Includes acknowledged replay of interrupted, uncommitted native units.
    pub dispatched_units: usize,
    pub current_unit: Option<usize>,
    pub revision: usize,
    pub resumes: usize,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub retention_seconds: u64,
    pub retryable: bool,
    pub retry_requires_uncommitted_acknowledgement: bool,
    pub failure: Option<Failure>,
    /// Always false: job state is metadata; completed artifacts contain scientific payloads.
    pub scientific_result: bool,
    pub instructions: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub unit: Unit,
    pub plan_hash: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    executor_sha256: String,
    submitted_sha256: String,
    tickets: Vec<Ticket>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    sha256: String,
    descriptor: Descriptor,
}
struct Entries {
    jobs: BTreeMap<String, Descriptor>,
    poisoned: bool,
}
pub struct Store {
    directory: PrivateDirectory,
    _lock: File,
    executor: String,
    entries: Mutex<Entries>,
}

fn storage(error: impl std::fmt::Display) -> Value {
    policy::error("local_job_storage", error.to_string())
}
pub fn schema() -> Value {
    let mut value = serde_json::to_value(schemars::schema_for!(Descriptor)).unwrap();
    value["properties"]["schema"]["const"] = json!("qspquest.local-job/v1");
    value["properties"]["scientific_result"]["const"] = json!(false);
    value
}
fn now() -> Result<u64, Value> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(storage)
}
fn identifier(value: &str) -> Result<(), Value> {
    if value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        Ok(())
    } else {
        Err(policy::error(
            "local_job_identifier",
            "Use the exact 64-character job_id returned by submit_local_job",
        ))
    }
}
pub fn submitted(input: &Submit) -> Result<(String, String), Value> {
    if input.idempotency_key.is_empty()
        || input.idempotency_key.len() > 128
        || !input
            .idempotency_key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:.".contains(&c))
    {
        return Err(policy::error(
            "local_job_idempotency",
            "idempotency_key must contain 1..128 ASCII letters, digits, -, _, : or .",
        ));
    }
    if !(1..=MAX_UNITS).contains(&input.work.len()) {
        return Err(policy::error(
            "local_job_work",
            "Supply 1..25 ordered work units",
        ));
    }
    let mut labels = std::collections::BTreeSet::new();
    for unit in &input.work {
        if unit.label.is_empty()
            || unit.label.len() > 120
            || unit.label.chars().any(char::is_control)
            || !labels.insert(&unit.label)
            || !unit.arguments.is_object()
        {
            return Err(policy::error(
                "local_job_work",
                "Work labels must be unique, nonempty, at most 120 bytes without controls; arguments must be objects",
            ));
        }
        if unit.arguments.get("save_csv").is_some() {
            return Err(policy::error(
                "local_job_export",
                "Local jobs retain immutable result artifacts; remove save_csv",
            ));
        }
    }
    let raw = serde_json::to_vec(input).map_err(storage)?;
    if raw.len() > policy::INPUT_LIMIT {
        return Err(policy::error("input_limit", "Local job input exceeds 1 MB"));
    }
    Ok((
        hash(format!("qspquest.local-job-id/v1:{}", input.idempotency_key).as_bytes()),
        hash(&raw),
    ))
}
fn read(directory: &PrivateDirectory, name: &str, limit: usize) -> Result<Vec<u8>, Value> {
    let file = directory.file(name, false, false, false).map_err(storage)?;
    if file.metadata().map_err(storage)?.len() > limit as u64 {
        return Err(storage("Local job file exceeds its bound"));
    }
    let mut raw = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(storage)?;
    if raw.len() > limit {
        return Err(storage("Local job file grew beyond its bound"));
    }
    Ok(raw)
}
fn write(directory: &PrivateDirectory, name: &str, raw: &[u8]) -> Result<(), Value> {
    let mut file = directory.file(name, true, true, true).map_err(storage)?;
    file.write_all(raw).map_err(storage)?;
    file.sync_all().map_err(storage)?;
    directory.sync().map_err(storage)
}
fn valid(descriptor: &Descriptor, executor: &str) -> Result<(), Value> {
    identifier(&descriptor.job_id)?;
    identifier(&descriptor.request_sha256)?;
    identifier(&descriptor.prepared_request_sha256)?;
    if descriptor.schema != "qspquest.local-job/v1"
        || descriptor.executor_sha256 != executor
        || descriptor.revision >= MAX_REVISIONS
        || !(1..=MAX_UNITS).contains(&descriptor.work_units)
        || descriptor.completed.len() > descriptor.work_units
        || descriptor.resumes > 3
        || descriptor.retention_seconds != RETENTION_SECONDS
        || descriptor.expires_at_unix_seconds
            != descriptor
                .created_at_unix_seconds
                .saturating_add(RETENTION_SECONDS)
        || descriptor.scientific_result
        || descriptor.dispatched_units < descriptor.completed.len()
        || descriptor
            .current_unit
            .is_some_and(|i| i != descriptor.completed.len() || i >= descriptor.work_units)
        || descriptor.completed.iter().enumerate().any(|(i, row)| {
            row.index != i
                || !row
                    .plan_hash
                    .strip_prefix("sha256:")
                    .is_some_and(|sha| identifier(sha).is_ok())
        })
        || (descriptor.state == State::Succeeded
            && (descriptor.completed.len() != descriptor.work_units
                || descriptor
                    .completed
                    .iter()
                    .any(|row| row.scientific_status != "success" || row.artifact.is_none())))
    {
        return Err(storage(
            "Local job descriptor, executable identity or ordered completion prefix is invalid",
        ));
    }
    Ok(())
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, Value> {
        let executable = std::env::current_exe().map_err(storage)?;
        let executor = hash(&std::fs::read(executable).map_err(storage)?);
        Self::open_for(path, executor)
    }
    fn open_for(path: &Path, executor: String) -> Result<Self, Value> {
        let directory = PrivateDirectory::open(path).map_err(storage)?;
        let names = directory.names().map_err(storage)?;
        if names.len() > MAX_NAMES
            || (!names.is_empty() && !names.iter().any(|name| name == MARKER))
        {
            return Err(storage(
                "Use an empty private directory dedicated to local jobs",
            ));
        }
        let (mut lock, new) = match directory.file(MARKER, true, true, true) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
                directory.file(MARKER, false, true, true).map_err(storage)?,
                false,
            ),
            Err(error) => return Err(storage(error)),
        };
        let magic = format!("qspquest.private-local-jobs/v1\n{executor}\n");
        if new {
            lock.write_all(magic.as_bytes()).map_err(storage)?;
            lock.sync_all().map_err(storage)?;
            directory.sync().map_err(storage)?;
        } else {
            let mut bytes = Vec::new();
            (&mut lock)
                .take(256)
                .read_to_end(&mut bytes)
                .map_err(storage)?;
            if bytes != magic.as_bytes() {
                return Err(storage(
                    "Local job directory belongs to a different executable; resume using the original binary or use a new directory",
                ));
            }
        }
        let mut latest: BTreeMap<String, usize> = BTreeMap::new();
        let mut total_bytes = 0u64;
        for name in &names {
            if name == MARKER {
                continue;
            }
            let (id, suffix) = name
                .split_once('-')
                .ok_or_else(|| storage("Unexpected local job filename"))?;
            identifier(id)?;
            if suffix == "request.json" || suffix == "expired.json" {
                total_bytes += read(&directory, name, MAX_REQUEST_BYTES)?.len() as u64;
                continue;
            }
            if suffix == "expired.commit" {
                read(&directory, name, 0)?;
                if !names.contains(&format!("{id}-expired.json")) {
                    return Err(storage("Expired job commit has no body"));
                }
                continue;
            }
            let revision = suffix
                .strip_suffix(".commit")
                .or_else(|| suffix.strip_suffix(".json"))
                .ok_or_else(|| storage("Unexpected job journal member"))?;
            let number = revision.parse::<usize>().map_err(storage)?;
            if number >= MAX_REVISIONS || revision != format!("{number:06}") {
                return Err(storage("Invalid local job journal revision"));
            }
            if suffix.ends_with(".commit") {
                if !read(&directory, name, 0)?.is_empty() {
                    return Err(storage("Invalid job commit marker"));
                }
                latest
                    .entry(id.to_owned())
                    .and_modify(|r| *r = (*r).max(number))
                    .or_insert(number);
            } else {
                total_bytes += read(&directory, name, MAX_STATE_BYTES)?.len() as u64;
            }
        }
        if total_bytes > 512_000_000
            || latest
                .keys()
                .filter(|id| !names.contains(&format!("{id}-expired.commit")))
                .count()
                > MAX_JOBS
        {
            return Err(storage("Local job storage exceeds its bounded capacity"));
        }
        let mut jobs = BTreeMap::new();
        for (id, revision) in latest {
            if names.contains(&format!("{id}-expired.commit")) {
                continue;
            }
            let envelope: Envelope = serde_json::from_slice(&read(
                &directory,
                &format!("{id}-{revision:06}.json"),
                MAX_STATE_BYTES,
            )?)
            .map_err(storage)?;
            if envelope.schema != "qspquest.local-job-journal/v1"
                || envelope.sha256
                    != hash(&serde_json::to_vec(&envelope.descriptor).map_err(storage)?)
                || envelope.descriptor.job_id != id
                || envelope.descriptor.revision != revision
            {
                return Err(storage("Job journal integrity or revision check failed"));
            }
            valid(&envelope.descriptor, &executor)?;
            jobs.insert(id, envelope.descriptor);
        }
        for name in &names {
            if let Some(id) = name.strip_suffix("-expired.json") {
                if !names.contains(&format!("{id}-expired.commit")) {
                    directory.remove(name).map_err(storage)?;
                    continue;
                }
                let descriptor: Descriptor =
                    serde_json::from_slice(&read(&directory, name, MAX_STATE_BYTES)?)
                        .map_err(storage)?;
                valid(&descriptor, &executor)?;
                if descriptor.job_id != id || descriptor.state != State::Expired {
                    return Err(storage("Invalid expired job marker"));
                }
                jobs.insert(id.to_owned(), descriptor);
            }
        }
        // A body without its empty commit marker was never published. Remove
        // only validated internal orphans, under the held directory lock.
        for name in &names {
            if let Some((id, _)) = name.split_once('-') {
                if jobs.get(id).is_some_and(|job| job.state == State::Expired)
                    && names.contains(&format!("{id}-expired.commit"))
                    && name != &format!("{id}-expired.json")
                    && name != &format!("{id}-expired.commit")
                {
                    directory.remove(name).map_err(storage)?;
                    continue;
                }
            }
            if name.ends_with(".json")
                && !name.ends_with("-expired.json")
                && !name.ends_with("-request.json")
            {
                let commit = name.strip_suffix(".json").unwrap().to_owned() + ".commit";
                if !names.contains(&commit) {
                    directory.remove(name).map_err(storage)?;
                }
            }
            if let Some(id) = name.strip_suffix("-request.json") {
                if !jobs.contains_key(id) {
                    directory.remove(name).map_err(storage)?;
                }
            }
        }
        let store = Self {
            directory,
            _lock: lock,
            executor,
            entries: Mutex::new(Entries {
                jobs,
                poisoned: false,
            }),
        };
        {
            let mut entries = store.entries.lock().unwrap();
            let interrupted: Vec<_> = entries
                .jobs
                .values()
                .filter(|job| matches!(job.state, State::Running | State::CancelRequested))
                .cloned()
                .collect();
            for mut job in interrupted {
                job.state = State::Failed;
                job.retryable = true;
                job.retry_requires_uncommitted_acknowledgement = job.current_unit.is_some();
                job.failure = Some(Failure { code: "local_job_interrupted".into(), message: "The prior process stopped; completed units remain committed. The in-flight unit may have run but has no committed result. Explicit retry_uncommitted is required before replay.".into() });
                store.change(&mut entries, job)?;
            }
            store.cleanup(&mut entries)?;
        }
        Ok(store)
    }
    fn healthy(entries: &Entries) -> Result<(), Value> {
        if entries.poisoned {
            Err(storage(
                "Job journal update failed. No further dispatch is permitted; reopen the same directory with the original executable to inspect committed state",
            ))
        } else {
            Ok(())
        }
    }
    fn persist(&self, job: &Descriptor) -> Result<(), Value> {
        valid(job, &self.executor)?;
        let envelope = Envelope {
            schema: "qspquest.local-job-journal/v1".into(),
            sha256: hash(&serde_json::to_vec(job).map_err(storage)?),
            descriptor: job.clone(),
        };
        let raw = serde_json::to_vec(&envelope).map_err(storage)?;
        if raw.len() > MAX_STATE_BYTES {
            return Err(storage("Job metadata exceeds 64 KB"));
        }
        write(
            &self.directory,
            &format!("{}-{:06}.json", job.job_id, job.revision),
            &raw,
        )?;
        // Creating this zero-byte file publishes a previously fsynced body.
        write(
            &self.directory,
            &format!("{}-{:06}.commit", job.job_id, job.revision),
            b"",
        )
    }
    fn change(&self, entries: &mut Entries, mut job: Descriptor) -> Result<Descriptor, Value> {
        Self::healthy(entries)?;
        job.revision += 1;
        if let Err(error) = self.persist(&job) {
            entries.poisoned = true;
            return Err(error);
        }
        entries.jobs.insert(job.job_id.clone(), job.clone());
        Ok(job)
    }
    fn cleanup(&self, entries: &mut Entries) -> Result<(), Value> {
        let result = self.cleanup_inner(entries);
        if result.is_err() {
            entries.poisoned = true;
        }
        result
    }
    fn cleanup_inner(&self, entries: &mut Entries) -> Result<(), Value> {
        Self::healthy(entries)?;
        let time = now()?;
        let names = self.directory.names().map_err(storage)?;
        let expired: Vec<_> = entries
            .jobs
            .values()
            .filter(|job| {
                job.expires_at_unix_seconds <= time
                    && !matches!(job.state, State::Running | State::CancelRequested)
                    && (job.state != State::Expired
                        || !names.contains(&format!("{}-expired.commit", job.job_id)))
            })
            .cloned()
            .collect();
        for mut job in expired {
            job.state = State::Expired;
            job.retryable = false;
            job.current_unit = None;
            job.completed.clear();
            job.failure = None;
            let name = format!("{}-expired.json", job.job_id);
            write(
                &self.directory,
                &name,
                &serde_json::to_vec(&job).map_err(storage)?,
            )?;
            let commit = format!("{}-expired.commit", job.job_id);
            write(&self.directory, &commit, b"")?;
            let files = self.directory.names().map_err(storage)?;
            for file in files.into_iter().filter(|file| {
                file.starts_with(&(job.job_id.clone() + "-")) && file != &name && file != &commit
            }) {
                self.directory.remove(&file).map_err(storage)?;
            }
            entries.jobs.insert(job.job_id.clone(), job);
        }
        let forget: Vec<_> = entries
            .jobs
            .values()
            .filter(|job| {
                job.state == State::Expired
                    && time
                        >= job
                            .expires_at_unix_seconds
                            .saturating_add(TOMBSTONE_SECONDS)
            })
            .map(|job| job.job_id.clone())
            .collect();
        for id in forget {
            self.directory
                .remove(&format!("{id}-expired.commit"))
                .map_err(storage)?;
            self.directory
                .remove(&format!("{id}-expired.json"))
                .map_err(storage)?;
            entries.jobs.remove(&id);
        }
        Ok(())
    }
    pub fn lookup(&self, id: &str, submitted_sha256: &str) -> Result<Option<Descriptor>, Value> {
        identifier(id)?;
        let entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        if let Some(job) = entries.jobs.get(id) {
            if job.request_sha256 != submitted_sha256 {
                return Err(policy::error(
                    "local_job_idempotency_conflict",
                    "This key already identifies different inputs; use the original input or a new key",
                ));
            }
            return Ok(Some(self.project(job.clone())?));
        }
        Ok(None)
    }
    fn project(&self, mut job: Descriptor) -> Result<Descriptor, Value> {
        if now()? >= job.expires_at_unix_seconds {
            job.state = State::Expired;
            job.retryable = false;
        }
        Ok(job)
    }
    pub fn get(&self, id: &str) -> Result<Descriptor, Value> {
        identifier(id)?;
        let entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        self.project(entries.jobs.get(id).cloned().ok_or_else(|| {
            policy::error(
                "local_job_not_found",
                "No job in this local directory has this id",
            )
        })?)
    }
    pub fn queued(&self) -> Result<Vec<String>, Value> {
        let entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        let time = now()?;
        Ok(entries
            .jobs
            .values()
            .filter(|job| job.state == State::Queued && job.expires_at_unix_seconds > time)
            .map(|job| job.job_id.clone())
            .collect())
    }
    pub fn enqueue(
        &self,
        id: String,
        digest: String,
        tickets: Vec<Ticket>,
    ) -> Result<(Descriptor, bool), Value> {
        let mut entries = self.entries.lock().unwrap();
        self.cleanup(&mut entries)?;
        if let Some(job) = entries.jobs.get(&id) {
            if job.request_sha256 != digest {
                return Err(policy::error(
                    "local_job_idempotency_conflict",
                    "This retry key already identifies different inputs",
                ));
            }
            return Ok((self.project(job.clone())?, false));
        }
        let live = entries
            .jobs
            .values()
            .filter(|job| job.state != State::Expired)
            .count();
        if live >= MAX_JOBS
            || entries.jobs.len() >= 256
            || entries
                .jobs
                .values()
                .filter(|job| job.state.active())
                .count()
                >= MAX_ACTIVE
        {
            return Err(policy::error(
                "local_job_capacity",
                "Local job capacity is full; existing work/results are preserved. Wait for completion/expiry or choose a new dedicated job directory",
            ));
        }
        let request = Request {
            schema: "qspquest.local-job-request/v1".into(),
            executor_sha256: self.executor.clone(),
            submitted_sha256: digest.clone(),
            tickets,
        };
        let raw = serde_json::to_vec(&request).map_err(storage)?;
        if raw.len() > MAX_REQUEST_BYTES {
            return Err(policy::error(
                "input_limit",
                "Resolved local job request exceeds 2 MB",
            ));
        }
        // Reserve worst-case journal space before dispatch. New admissions can
        // never consume the metadata space needed to finish existing jobs.
        let mut used = 0u64;
        for name in self.directory.names().map_err(storage)? {
            if name != MARKER {
                used += self
                    .directory
                    .file(&name, false, false, false)
                    .map_err(storage)?
                    .metadata()
                    .map_err(storage)?
                    .len();
            }
        }
        let reserved: u64 = entries
            .jobs
            .values()
            .filter(|job| job.state != State::Expired)
            .map(|job| (MAX_REVISIONS - job.revision) as u64 * MAX_STATE_BYTES as u64)
            .sum();
        if used + reserved + raw.len() as u64 + (MAX_REVISIONS as u64 + 1) * MAX_STATE_BYTES as u64
            > 512_000_000
        {
            return Err(policy::error(
                "local_job_capacity",
                "Job storage reservations are full; existing jobs can still finish. Wait for input/journal expiry or choose a new directory",
            ));
        }
        let time = now()?;
        let job = Descriptor { schema: "qspquest.local-job/v1".into(), job_id: id.clone(), state: State::Queued,
            request_sha256: digest, prepared_request_sha256: hash(&raw), executor_sha256: self.executor.clone(), work_units: request.tickets.len(), completed: vec![],
            dispatched_units: 0, current_unit: None, revision: 0, resumes: 0, created_at_unix_seconds: time,
            expires_at_unix_seconds: time.saturating_add(RETENTION_SECONDS), retention_seconds: RETENTION_SECONDS,
            retryable: false, retry_requires_uncommitted_acknowledgement: false, failure: None, scientific_result: false,
            instructions: "Admission and job state are metadata, not a scientific result. Read every completed unit artifact in order and verify its bytes/digest, identity and scientific status. Cancellation drains the current bounded native unit. Resume skips committed units. Artifact expiry is independent; no mid-solve checkpoint is available. Job inputs/journals expire 24 hours after admission; bounded expiration markers retain the retry key digest for seven further days.".into() };
        if let Err(error) = write(&self.directory, &format!("{id}-request.json"), &raw)
            .and_then(|_| self.persist(&job))
        {
            entries.poisoned = true;
            return Err(error);
        }
        entries.jobs.insert(id, job.clone());
        Ok((job, true))
    }
    pub fn tickets(&self, id: &str) -> Result<Vec<Ticket>, Value> {
        let job = self.get(id)?;
        if job.state == State::Expired {
            return Err(policy::error(
                "local_job_expired",
                "Job input retention expired; start a new experiment with a new key",
            ));
        }
        let raw = read(
            &self.directory,
            &format!("{id}-request.json"),
            MAX_REQUEST_BYTES,
        )?;
        if hash(&raw) != job.prepared_request_sha256 {
            return Err(storage("Stored local job request bytes changed"));
        }
        let request: Request = serde_json::from_slice(&raw).map_err(storage)?;
        if request.schema != "qspquest.local-job-request/v1"
            || request.executor_sha256 != self.executor
            || request.submitted_sha256 != job.request_sha256
            || request.tickets.len() != job.work_units
        {
            return Err(storage("Stored job request identity differs"));
        }
        Ok(request.tickets)
    }
    pub fn begin(&self, id: &str) -> Result<bool, Value> {
        let mut entries = self.entries.lock().unwrap();
        self.cleanup(&mut entries)?;
        let Some(mut job) = entries.jobs.get(id).cloned() else {
            return Ok(false);
        };
        if job.state != State::Queued || now()? >= job.expires_at_unix_seconds {
            return Ok(false);
        }
        job.state = State::Running;
        self.change(&mut entries, job)?;
        Ok(true)
    }
    pub fn start_unit(&self, id: &str, index: usize) -> Result<bool, Value> {
        let mut entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        let mut job = entries
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| storage("Running local job disappeared"))?;
        if job.state != State::Running || now()? >= job.expires_at_unix_seconds {
            return Ok(false);
        }
        if index != job.completed.len() || index >= job.work_units || job.current_unit.is_some() {
            return Err(storage("Job unit dispatch order differs"));
        }
        job.dispatched_units += 1;
        job.current_unit = Some(index);
        self.change(&mut entries, job)?;
        Ok(true)
    }
    pub fn finish_unit(&self, id: &str, unit: CompletedUnit) -> Result<(), Value> {
        let mut entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        let mut job = entries
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| storage("Running local job disappeared"))?;
        if job.current_unit != Some(unit.index) || unit.index != job.completed.len() {
            return Err(storage("Job completion order differs"));
        }
        let failed = unit.scientific_status != "success";
        job.completed.push(unit);
        job.current_unit = None;
        if failed {
            job.state = State::Failed;
            job.retryable = false;
            job.failure = job.completed.last().and_then(|unit| unit.failure.clone());
        }
        self.change(&mut entries, job)?;
        Ok(())
    }
    pub fn finish(&self, id: &str, failure: Option<Failure>) -> Result<Descriptor, Value> {
        let mut entries = self.entries.lock().unwrap();
        Self::healthy(&entries)?;
        let mut job = entries
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| storage("Running local job disappeared"))?;
        if now()? >= job.expires_at_unix_seconds {
            job.state = State::Expired;
            job.retryable = false;
        } else if let Some(failure) = failure {
            job.state = State::Failed;
            job.failure = Some(failure);
            job.retryable = false;
        } else if job.state == State::Failed {
            return Ok(job);
        } else if job.completed.len() == job.work_units {
            job.state = State::Succeeded;
            job.retryable = false;
        } else {
            job.state = State::Cancelled;
            job.retryable = true;
        }
        job.current_unit = None;
        self.change(&mut entries, job)
    }
    pub fn cancel(&self, id: &str) -> Result<Descriptor, Value> {
        identifier(id)?;
        let mut entries = self.entries.lock().unwrap();
        self.cleanup(&mut entries)?;
        let mut job = entries
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| policy::error("local_job_not_found", "Local job not found"))?;
        match job.state {
            State::Queued => {
                job.state = State::Cancelled;
                job.retryable = true;
            }
            State::Running => job.state = State::CancelRequested,
            _ => return self.project(job),
        }
        self.change(&mut entries, job)
    }
    pub fn resume(&self, id: &str, retry_uncommitted: bool) -> Result<(Descriptor, bool), Value> {
        identifier(id)?;
        let mut entries = self.entries.lock().unwrap();
        self.cleanup(&mut entries)?;
        let mut job = entries
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| policy::error("local_job_not_found", "Local job not found"))?;
        if job.state.active() || job.state == State::Succeeded {
            return Ok((self.project(job)?, false));
        }
        if job.state == State::Expired {
            return Err(policy::error(
                "local_job_expired",
                "Job retention expired; start a new experiment with a new key",
            ));
        }
        if !job.retryable || job.resumes >= 3 {
            return Err(policy::error(
                "local_job_not_retryable",
                "This job cannot resume. Scientific/request failures need corrected inputs and a new key; at most three resumes are allowed",
            ));
        }
        if job.retry_requires_uncommitted_acknowledgement && !retry_uncommitted {
            return Err(policy::error(
                "local_job_uncommitted_retry",
                "The interrupted native unit may already have executed. Set retry_uncommitted:true to replay only that uncommitted unit; completed units will be skipped",
            ));
        }
        if entries
            .jobs
            .values()
            .filter(|job| job.state.active())
            .count()
            >= MAX_ACTIVE
        {
            return Err(policy::error(
                "local_job_capacity",
                "The active local job queue is full",
            ));
        }
        if job.revision + 2 * (job.work_units - job.completed.len()) + 10 >= MAX_REVISIONS {
            return Err(storage("Job journal revision capacity exhausted"));
        }
        job.state = State::Queued;
        job.current_unit = None;
        job.resumes += 1;
        job.retryable = false;
        job.retry_requires_uncommitted_acknowledgement = false;
        job.failure = None;
        Ok((self.change(&mut entries, job)?, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            Self(std::env::temp_dir().join(
                format!("qspquest-job-test-{}-{}-{}", std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            ))
        }
        fn store(&self) -> Store {
            Store::open_for(&self.0, "a".repeat(64)).unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn input(key: &str, count: usize) -> Submit {
        Submit {
            idempotency_key: key.into(),
            work: (0..count)
                .map(|index| Unit {
                    label: format!("condition-{index}"),
                    operation: Operation::Simulate,
                    arguments: json!({"model":{"source":"synthetic"}}),
                })
                .collect(),
        }
    }
    fn enqueue(store: &Store, input: &Submit) -> Descriptor {
        let (id, sha) = submitted(input).unwrap();
        store
            .enqueue(
                id,
                sha,
                input
                    .work
                    .iter()
                    .cloned()
                    .map(|unit| Ticket {
                        unit,
                        plan_hash: format!("sha256:{}", "b".repeat(64)),
                    })
                    .collect(),
            )
            .unwrap()
            .0
    }
    fn row(index: usize) -> CompletedUnit {
        CompletedUnit {
            index,
            label: format!("condition-{index}"),
            operation: "simulate".into(),
            plan_hash: format!("sha256:{}", "b".repeat(64)),
            scientific_status: "success".into(),
            failure: None,
            artifact: Some(artifacts::Descriptor {
                schema: artifacts::Version::V01,
                uri: format!("{}{}", artifacts::URI_PREFIX, "c".repeat(64)),
                sha256: "d".repeat(64),
                bytes: 2,
                media_type: "application/json".into(),
                operation: "simulate".into(),
                result_schema: None,
                created_at_unix_seconds: 1,
                expires_at_unix_seconds: 86401,
                retention_seconds: artifacts::RETENTION_SECONDS,
                chunk_bytes: artifacts::CHUNK_BYTES,
                chunk_count: 1,
                chunk_uri_prefix: "synthetic".into(),
                instructions: "synthetic journal fixture".into(),
            }),
        }
    }
    #[test]
    fn cancellation_drains_one_unit_and_resume_skips_the_committed_prefix() {
        let dir = Scratch::new();
        let store = dir.store();
        let request = input("cancel-resume", 2);
        let id = enqueue(&store, &request).job_id;
        assert!(store.begin(&id).unwrap());
        assert!(store.start_unit(&id, 0).unwrap());
        assert_eq!(store.cancel(&id).unwrap().state, State::CancelRequested);
        assert!(!store.start_unit(&id, 1).unwrap());
        store.finish_unit(&id, row(0)).unwrap();
        assert_eq!(store.finish(&id, None).unwrap().state, State::Cancelled);
        assert!(store.resume(&id, false).unwrap().1);
        assert!(store.begin(&id).unwrap());
        assert!(store.start_unit(&id, 0).is_err());
        assert!(store.start_unit(&id, 1).unwrap());
        store.finish_unit(&id, row(1)).unwrap();
        let done = store.finish(&id, None).unwrap();
        assert_eq!(done.state, State::Succeeded);
        assert_eq!(done.dispatched_units, 2);
        assert!(!store.resume(&id, false).unwrap().1);
    }
    #[test]
    fn queued_cancellation_and_retry_keys_never_dispatch_duplicate_work() {
        let dir = Scratch::new();
        let store = dir.store();
        let request = input("stable", 1);
        let first = enqueue(&store, &request);
        let again = enqueue(&store, &request);
        assert_eq!(first.job_id, again.job_id);
        assert_eq!(again.revision, 0);
        assert_eq!(store.cancel(&first.job_id).unwrap().state, State::Cancelled);
        assert!(!store.begin(&first.job_id).unwrap());
        assert_eq!(store.get(&first.job_id).unwrap().dispatched_units, 0);
        let mut changed = input("stable", 1);
        changed.work[0].arguments["changed"] = json!(true);
        let (id, sha) = submitted(&changed).unwrap();
        assert_eq!(
            store.lookup(&id, &sha).unwrap_err()["error"]["code"],
            "local_job_idempotency_conflict"
        );
    }
    #[test]
    fn interrupted_dispatch_requires_acknowledgement_but_never_replays_committed_units() {
        let dir = Scratch::new();
        let store = dir.store();
        let id = enqueue(&store, &input("restart", 2)).job_id;
        assert!(store.begin(&id).unwrap());
        assert!(store.start_unit(&id, 0).unwrap());
        store.finish_unit(&id, row(0)).unwrap();
        assert!(store.start_unit(&id, 1).unwrap());
        assert!(Store::open_for(&dir.0, "a".repeat(64)).is_err());
        drop(store);
        let store = dir.store();
        let interrupted = store.get(&id).unwrap();
        assert_eq!(interrupted.state, State::Failed);
        assert!(interrupted.retry_requires_uncommitted_acknowledgement);
        assert!(store.resume(&id, false).is_err());
        assert!(store.resume(&id, true).unwrap().1);
        assert!(store.begin(&id).unwrap());
        assert!(store.start_unit(&id, 0).is_err());
        assert!(store.start_unit(&id, 1).unwrap());
        store.finish_unit(&id, row(1)).unwrap();
        assert_eq!(store.finish(&id, None).unwrap().dispatched_units, 3);
    }
    #[test]
    fn published_corruption_fails_closed_and_uncommitted_orphans_do_not_publish() {
        let dir = Scratch::new();
        let store = dir.store();
        let id = enqueue(&store, &input("integrity", 1)).job_id;
        write(
            &store.directory,
            &format!("{id}-000001.json"),
            b"incomplete",
        )
        .unwrap();
        drop(store);
        let store = dir.store();
        assert_eq!(store.get(&id).unwrap().revision, 0);
        assert!(!dir.0.join(format!("{id}-000001.json")).exists());
        drop(store);
        std::fs::write(dir.0.join(format!("{id}-000000.json")), b"corrupt").unwrap();
        assert!(Store::open_for(&dir.0, "a".repeat(64)).is_err());
    }
    #[test]
    fn input_expiry_preserves_a_retry_tombstone_and_removes_input_journals() {
        let dir = Scratch::new();
        let store = dir.store();
        let request = input("expiry", 1);
        let mut job = enqueue(&store, &request);
        job.created_at_unix_seconds = now().unwrap() - RETENTION_SECONDS - 1;
        job.expires_at_unix_seconds = job.created_at_unix_seconds + RETENTION_SECONDS;
        {
            let mut entries = store.entries.lock().unwrap();
            store.change(&mut entries, job.clone()).unwrap();
            store.cleanup(&mut entries).unwrap();
        }
        assert!(!dir.0.join(format!("{}-request.json", job.job_id)).exists());
        drop(store);
        let store = dir.store();
        assert_eq!(store.get(&job.job_id).unwrap().state, State::Expired);
        assert_eq!(enqueue(&store, &request).state, State::Expired);
        assert!(store.resume(&job.job_id, true).is_err());
        assert!(store.tickets(&job.job_id).is_err());
    }
    #[test]
    fn scientific_failure_is_retained_and_cannot_be_retried_as_success() {
        let dir = Scratch::new();
        let store = dir.store();
        let id = enqueue(&store, &input("nonconverged", 2)).job_id;
        assert!(store.begin(&id).unwrap());
        assert!(store.start_unit(&id, 0).unwrap());
        let mut failed = row(0);
        failed.scientific_status = "scientific_failure".into();
        failed.failure = Some(Failure {
            code: "nonconverged".into(),
            message: "Synthetic failed fit".into(),
        });
        store.finish_unit(&id, failed).unwrap();
        let final_state = store.finish(&id, None).unwrap();
        assert_eq!(final_state.state, State::Failed);
        assert!(final_state.completed[0].artifact.is_some());
        assert!(!store.start_unit(&id, 1).unwrap());
        assert!(store.resume(&id, true).is_err());
    }
    #[test]
    fn admission_capacity_preserves_existing_work_and_changed_request_bytes_refuse_dispatch() {
        let dir = Scratch::new();
        let store = dir.store();
        let first = enqueue(&store, &input("first", 1));
        for index in 1..MAX_ACTIVE {
            enqueue(&store, &input(&format!("queued-{index}"), 1));
        }
        let overflow = input("over-capacity", 1);
        let (id, sha) = submitted(&overflow).unwrap();
        let tickets = overflow
            .work
            .into_iter()
            .map(|unit| Ticket {
                unit,
                plan_hash: format!("sha256:{}", "b".repeat(64)),
            })
            .collect();
        assert_eq!(
            store.enqueue(id, sha, tickets).unwrap_err()["error"]["code"],
            "local_job_capacity"
        );
        assert_eq!(store.queued().unwrap().len(), MAX_ACTIVE);
        assert_eq!(store.get(&first.job_id).unwrap().dispatched_units, 0);
        std::fs::write(dir.0.join(format!("{}-request.json", first.job_id)), b"{}").unwrap();
        assert!(store.tickets(&first.job_id).is_err());
    }
    #[test]
    fn executable_changes_and_unexpected_directory_entries_are_rejected() {
        let dir = Scratch::new();
        drop(dir.store());
        assert!(Store::open_for(&dir.0, "e".repeat(64)).is_err());
        std::fs::write(dir.0.join("unrelated.txt"), b"preserve").unwrap();
        assert!(Store::open_for(&dir.0, "a".repeat(64)).is_err());
        assert_eq!(
            std::fs::read(dir.0.join("unrelated.txt")).unwrap(),
            b"preserve"
        );
    }
}
