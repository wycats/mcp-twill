use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    future::Future,
    hash::{Hash, Hasher},
    io,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

pub type TaskStoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub const TASK_RUNTIME_CONTRACT_VERSION: u32 = 1;
pub const MAX_STORED_TASK_RECORD_BYTES: usize = 1_048_576;
pub const MAX_STORED_TASKS: usize = 256;
pub(crate) const MAX_TASK_TIMESTAMP_MILLIS: u64 = 253_402_300_799_999;

static SERVER_INSTANCE_MOUNTS: OnceLock<StdMutex<HashSet<usize>>> = OnceLock::new();

pub trait TaskStore: Send + Sync + 'static {
    fn scope(&self) -> TaskStoreScope;

    fn create(
        &self,
        key: TaskStorageKey,
        record: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreCreate, TaskStoreError>>;

    fn get(
        &self,
        key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<Option<StoredTaskRecord>, TaskStoreError>>;

    fn compare_and_set(
        &self,
        key: TaskStorageKey,
        expected: TaskRevision,
        next: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreWrite, TaskStoreError>>;

    fn remove(
        &self,
        key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreRemoval, TaskStoreError>>;
}

pub struct InMemoryTaskStore {
    scope: TaskStoreScope,
    records: Mutex<HashMap<TaskStorageKey, StoredTaskRecord>>,
}

impl InMemoryTaskStore {
    pub fn connection() -> Arc<Self> {
        Arc::new(Self::new(TaskStoreScope::Connection))
    }

    pub fn server_instance() -> Arc<Self> {
        Arc::new(Self::new(TaskStoreScope::ServerInstance))
    }

    fn new(scope: TaskStoreScope) -> Self {
        Self {
            scope,
            records: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) async fn record_count_for_test(&self) -> usize {
        self.records.lock().await.len()
    }
}

impl TaskStore for InMemoryTaskStore {
    fn scope(&self) -> TaskStoreScope {
        self.scope
    }

    fn create(
        &self,
        key: TaskStorageKey,
        record: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreCreate, TaskStoreError>> {
        Box::pin(async move {
            let mut records = self.records.lock().await;
            if records.contains_key(&key) {
                return Ok(TaskStoreCreate::Occupied);
            }
            if records.len() >= MAX_STORED_TASKS {
                return Ok(TaskStoreCreate::CapacityExceeded);
            }
            records.insert(key, record);
            Ok(TaskStoreCreate::Created)
        })
    }

    fn get(
        &self,
        key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<Option<StoredTaskRecord>, TaskStoreError>> {
        Box::pin(async move { Ok(self.records.lock().await.get(&key).cloned()) })
    }

    fn compare_and_set(
        &self,
        key: TaskStorageKey,
        expected: TaskRevision,
        next: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreWrite, TaskStoreError>> {
        Box::pin(async move {
            let mut records = self.records.lock().await;
            let Some(current) = records.get(&key) else {
                return Ok(TaskStoreWrite::Missing);
            };
            if current.revision() != expected
                || expected.0 == u64::MAX
                || next.revision().0 != expected.0 + 1
            {
                return Ok(TaskStoreWrite::Conflict);
            }
            records.insert(key, next);
            Ok(TaskStoreWrite::Written)
        })
    }

    fn remove(
        &self,
        key: TaskStorageKey,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreRemoval, TaskStoreError>> {
        Box::pin(async move {
            Ok(if self.records.lock().await.remove(&key).is_some() {
                TaskStoreRemoval::Removed
            } else {
                TaskStoreRemoval::Missing
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreScope {
    Connection,
    ServerInstance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreCreate {
    Created,
    Occupied,
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreWrite {
    Written,
    Conflict,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreRemoval {
    Removed,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskStorageKey([u8; 32]);

impl TaskStorageKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TaskStorageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskStorageKey(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskRevision(u64);

impl TaskRevision {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskExpiration(u64);

impl TaskExpiration {
    pub fn unix_millis(self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
pub struct StoredTaskRecord {
    bytes: Box<[u8]>,
    revision: TaskRevision,
    expires_at: Option<TaskExpiration>,
}

impl StoredTaskRecord {
    pub fn revision(&self) -> TaskRevision {
        self.revision
    }

    pub fn expires_at(&self) -> Option<TaskExpiration> {
        self.expires_at
    }

    pub fn storage_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_storage_bytes(self) -> Box<[u8]> {
        self.bytes
    }

    pub fn from_storage_bytes(
        bytes: impl Into<Box<[u8]>>,
    ) -> std::result::Result<Self, TaskRecordCodecError> {
        let bytes = bytes.into();
        if bytes.len() > MAX_STORED_TASK_RECORD_BYTES {
            return Err(TaskRecordCodecError::new());
        }
        let record: SemanticTaskRecord =
            serde_json::from_slice(&bytes).map_err(|_| TaskRecordCodecError::new())?;
        record.validate()?;
        Ok(Self {
            revision: TaskRevision(record.revision),
            expires_at: record.expires_at.map(TaskExpiration),
            bytes,
        })
    }

    pub(crate) fn encode(
        record: &SemanticTaskRecord,
    ) -> std::result::Result<Self, TaskRecordCodecError> {
        record.validate()?;
        let mut writer = BoundedRecordWriter::default();
        serde_json::to_writer(&mut writer, record).map_err(|_| TaskRecordCodecError::new())?;
        let bytes = writer.bytes;
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            revision: TaskRevision(record.revision),
            expires_at: record.expires_at.map(TaskExpiration),
        })
    }

    pub(crate) fn decode(&self) -> std::result::Result<SemanticTaskRecord, TaskRecordCodecError> {
        let record: SemanticTaskRecord =
            serde_json::from_slice(&self.bytes).map_err(|_| TaskRecordCodecError::new())?;
        record.validate()?;
        Ok(record)
    }
}

#[derive(Default)]
struct BoundedRecordWriter {
    bytes: Vec<u8>,
}

impl io::Write for BoundedRecordWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = MAX_STORED_TASK_RECORD_BYTES.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            return Err(io::Error::other("task record exceeds storage bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Debug for StoredTaskRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredTaskRecord")
            .field("bytes", &"<redacted>")
            .field("revision", &self.revision)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub struct TaskRecordCodecError;

impl TaskRecordCodecError {
    fn new() -> Self {
        Self
    }
}

impl fmt::Debug for TaskRecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskRecordCodecError(<redacted>)")
    }
}

impl fmt::Display for TaskRecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("task record codec failed")
    }
}

impl Error for TaskRecordCodecError {}

pub struct TaskStoreError {
    _source: Box<dyn Error + Send + Sync + 'static>,
}

impl TaskStoreError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            _source: Box::new(source),
        }
    }
}

impl fmt::Debug for TaskStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskStoreError(<redacted>)")
    }
}

impl fmt::Display for TaskStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("task store failed")
    }
}

impl Error for TaskStoreError {}

pub enum TaskAccessPolicy {
    CapabilityId,
    Scoped(Arc<dyn TaskAccessScopeProvider>),
}

impl Clone for TaskAccessPolicy {
    fn clone(&self) -> Self {
        match self {
            Self::CapabilityId => Self::CapabilityId,
            Self::Scoped(provider) => Self::Scoped(provider.clone()),
        }
    }
}

impl fmt::Debug for TaskAccessPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapabilityId => formatter.write_str("CapabilityId"),
            Self::Scoped(_) => formatter.write_str("Scoped(<redacted>)"),
        }
    }
}

pub struct TaskAccessScope([u8; 32]);

impl TaskAccessScope {
    pub fn new(principal: impl AsRef<[u8]>) -> std::result::Result<Self, TaskAccessScopeError> {
        let principal = principal.as_ref();
        if !(1..=4096).contains(&principal.len()) {
            return Err(TaskAccessScopeError::invalid());
        }
        let mut digest = Sha256::new();
        digest.update(b"io.github.wycats.mcp-twill/task-access-scope");
        digest.update([0]);
        digest.update((principal.len() as u64).to_be_bytes());
        digest.update(principal);
        Ok(Self(digest.finalize().into()))
    }
}

impl PartialEq for TaskAccessScope {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

impl Eq for TaskAccessScope {}

impl fmt::Debug for TaskAccessScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskAccessScope(<redacted>)")
    }
}

pub struct TaskAccessScopeError {
    _source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl TaskAccessScopeError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            _source: Some(Box::new(source)),
        }
    }

    fn invalid() -> Self {
        Self { _source: None }
    }
}

impl fmt::Debug for TaskAccessScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskAccessScopeError(<redacted>)")
    }
}

impl fmt::Display for TaskAccessScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("task access scope failed")
    }
}

impl Error for TaskAccessScopeError {}

pub struct TaskAccessContext<'a> {
    transport_extensions: &'a rmcp::model::Extensions,
}

impl<'a> TaskAccessContext<'a> {
    pub fn transport_extensions(&self) -> &'a rmcp::model::Extensions {
        self.transport_extensions
    }

    pub(crate) fn new(transport_extensions: &'a rmcp::model::Extensions) -> Self {
        Self {
            transport_extensions,
        }
    }
}

impl fmt::Debug for TaskAccessContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskAccessContext(<redacted>)")
    }
}

pub trait TaskAccessScopeProvider: Send + Sync + 'static {
    fn scope(
        &self,
        context: TaskAccessContext<'_>,
    ) -> std::result::Result<TaskAccessScope, TaskAccessScopeError>;
}

#[derive(Clone)]
pub(crate) struct TaskRuntime {
    pub(crate) store: Arc<dyn TaskStore>,
    pub(crate) access: TaskAccessPolicy,
    pub(crate) _mount: Option<Arc<TaskStoreMount>>,
}

impl fmt::Debug for TaskRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskRuntime")
            .field("store", &"<private>")
            .field("access", &self.access)
            .finish()
    }
}

pub(crate) struct TaskStoreMount {
    identity: usize,
}

impl TaskStoreMount {
    pub(crate) fn acquire(store: &Arc<dyn TaskStore>) -> std::result::Result<Arc<Self>, ()> {
        let identity = Arc::as_ptr(store) as *const () as usize;
        let mut mounts = SERVER_INSTANCE_MOUNTS
            .get_or_init(|| StdMutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !mounts.insert(identity) {
            return Err(());
        }
        Ok(Arc::new(Self { identity }))
    }
}

impl Drop for TaskStoreMount {
    fn drop(&mut self) {
        SERVER_INSTANCE_MOUNTS
            .get_or_init(|| StdMutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.identity);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SemanticTaskStatus {
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SemanticTaskProfile {
    Legacy2025_11_25,
    TasksExtension,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticTaskRecord {
    version: u32,
    task_id: String,
    surface_hash: String,
    access_tag: u8,
    scope: Option<String>,
    profile: SemanticTaskProfile,
    created_at: u64,
    updated_at: u64,
    expires_at: Option<u64>,
    revision: u64,
    status: SemanticTaskStatus,
    outcome: Option<Value>,
}

impl SemanticTaskRecord {
    pub(crate) fn working(
        task_id: String,
        surface_hash: String,
        access_tag: u8,
        scope: Option<String>,
        profile: SemanticTaskProfile,
        now: u64,
        expires_at: Option<u64>,
    ) -> Self {
        Self {
            version: TASK_RUNTIME_CONTRACT_VERSION,
            task_id,
            surface_hash,
            access_tag,
            scope,
            profile,
            created_at: now,
            updated_at: now,
            expires_at,
            revision: 0,
            status: SemanticTaskStatus::Working,
            outcome: None,
        }
    }

    pub(crate) fn successor(
        &self,
        status: SemanticTaskStatus,
        outcome: Option<Value>,
        now: u64,
    ) -> Self {
        let mut next = self.clone();
        next.revision = next.revision.saturating_add(1);
        next.updated_at = now;
        next.status = status;
        next.outcome = outcome;
        next
    }

    pub(crate) fn task_id(&self) -> &str {
        &self.task_id
    }

    pub(crate) fn surface_hash(&self) -> &str {
        &self.surface_hash
    }

    pub(crate) fn access_tag(&self) -> u8 {
        self.access_tag
    }

    pub(crate) fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    pub(crate) fn created_at(&self) -> u64 {
        self.created_at
    }

    pub(crate) fn profile(&self) -> SemanticTaskProfile {
        self.profile
    }

    pub(crate) fn updated_at(&self) -> u64 {
        self.updated_at
    }

    pub(crate) fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    pub(crate) fn status(&self) -> SemanticTaskStatus {
        self.status
    }

    pub(crate) fn outcome(&self) -> Option<&Value> {
        self.outcome.as_ref()
    }

    fn validate(&self) -> std::result::Result<(), TaskRecordCodecError> {
        if self.version != TASK_RUNTIME_CONTRACT_VERSION
            || decode_hex::<32>(&self.task_id).is_none()
            || decode_hex::<32>(&self.surface_hash).is_none()
            || !matches!(self.access_tag, 0 | 1)
            || (self.access_tag == 0 && self.scope.is_some())
            || (self.access_tag == 1 && self.scope.as_deref().and_then(decode_hex::<32>).is_none())
            || self.updated_at < self.created_at
            || self.created_at > MAX_TASK_TIMESTAMP_MILLIS
            || self.updated_at > MAX_TASK_TIMESTAMP_MILLIS
            || self
                .expires_at
                .is_some_and(|deadline| deadline > MAX_TASK_TIMESTAMP_MILLIS)
            || self
                .expires_at
                .is_some_and(|deadline| deadline < self.created_at)
            || (matches!(self.status, SemanticTaskStatus::Working) != self.outcome.is_none())
            || !valid_outcome(self.profile, self.status, self.outcome.as_ref())
        {
            return Err(TaskRecordCodecError::new());
        }
        Ok(())
    }
}

fn valid_outcome(
    profile: SemanticTaskProfile,
    status: SemanticTaskStatus,
    outcome: Option<&Value>,
) -> bool {
    match (profile, status, outcome) {
        (_, SemanticTaskStatus::Working, None) => true,
        (
            SemanticTaskProfile::Legacy2025_11_25,
            SemanticTaskStatus::Completed | SemanticTaskStatus::Failed,
            Some(outcome),
        ) => valid_tool_outcome(outcome, false) || valid_json_rpc_error(outcome),
        (SemanticTaskProfile::TasksExtension, SemanticTaskStatus::Completed, Some(outcome)) => {
            valid_tool_outcome(outcome, true)
        }
        (SemanticTaskProfile::TasksExtension, SemanticTaskStatus::Failed, Some(outcome)) => {
            valid_json_rpc_error(outcome)
        }
        (_, SemanticTaskStatus::Cancelled, Some(outcome)) => {
            outcome.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
                && outcome.pointer("/error/code").and_then(Value::as_i64) == Some(-32000)
                && outcome.pointer("/error/message").and_then(Value::as_str)
                    == Some("Task cancelled")
        }
        _ => false,
    }
}

fn valid_tool_outcome(outcome: &Value, require_result_type: bool) -> bool {
    let Some(outcome) = outcome.as_object() else {
        return false;
    };
    outcome.get("content").is_some_and(Value::is_array)
        && (!require_result_type
            || outcome.get("resultType").and_then(Value::as_str) == Some("complete"))
}

fn valid_json_rpc_error(outcome: &Value) -> bool {
    outcome.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && outcome
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .is_some()
        && outcome
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some()
}

pub(crate) fn checked_task_expiration(created_at: u64, ttl: u64) -> Option<u64> {
    created_at
        .checked_add(ttl)
        .filter(|deadline| *deadline <= MAX_TASK_TIMESTAMP_MILLIS)
}

pub(crate) fn generate_task_id() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    encode_hex(&bytes)
}

pub(crate) fn derive_task_storage_key(
    surface_hash: &str,
    access: &TaskAccessPolicy,
    task_id: &str,
    scope: Option<&TaskAccessScope>,
) -> Option<TaskStorageKey> {
    let surface_hash = decode_hex::<32>(surface_hash)?;
    let task_id = decode_hex::<32>(task_id)?;
    let (access_tag, scope) = match (access, scope) {
        (TaskAccessPolicy::CapabilityId, None) => (0_u8, None),
        (TaskAccessPolicy::Scoped(_), Some(scope)) => (1_u8, Some(scope.0)),
        _ => return None,
    };
    let mut digest = Sha256::new();
    digest.update(b"io.github.wycats.mcp-twill/task-storage-key");
    digest.update([0]);
    digest.update(surface_hash);
    digest.update([access_tag]);
    digest.update(task_id);
    if let Some(scope) = scope {
        digest.update(scope);
    }
    Some(TaskStorageKey(digest.finalize().into()))
}

pub(crate) fn scope_hex(scope: &TaskAccessScope) -> String {
    encode_hex(&scope.0)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_nibble(pair[0])?;
        let low = decode_nibble(pair[1])?;
        output[index] = (high << 4) | low;
    }
    Some(output)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

impl Hash for TaskAccessScope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn working() -> SemanticTaskRecord {
        SemanticTaskRecord::working(
            "11".repeat(32),
            "22".repeat(32),
            0,
            None,
            SemanticTaskProfile::TasksExtension,
            1_700_000_000_000,
            Some(1_700_000_060_000),
        )
    }

    #[test]
    fn version_one_codec_is_canonical_bounded_and_redacted() {
        let working = working();
        let stored = StoredTaskRecord::encode(&working).unwrap();
        assert_eq!(stored.revision().get(), 0);
        assert_eq!(
            stored.expires_at().unwrap().unix_millis(),
            1_700_000_060_000
        );
        assert_eq!(
            stored.decode().unwrap().status(),
            SemanticTaskStatus::Working
        );
        assert_eq!(
            std::str::from_utf8(stored.storage_bytes()).unwrap(),
            concat!(
                "{\"version\":1,\"taskId\":\"",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "\",\"surfaceHash\":\"",
                "2222222222222222222222222222222222222222222222222222222222222222",
                "\",\"accessTag\":0,\"scope\":null,\"profile\":\"tasksExtension\",",
                "\"createdAt\":1700000000000,\"updatedAt\":1700000000000,",
                "\"expiresAt\":1700000060000,\"revision\":0,\"status\":\"working\",",
                "\"outcome\":null}"
            )
        );
        assert!(!format!("{stored:?}").contains("111111"));

        let completed = working.successor(
            SemanticTaskStatus::Completed,
            Some(json!({
                "content": [],
                "resultType": "complete",
                "private": "result"
            })),
            1_700_000_000_001,
        );
        let completed = StoredTaskRecord::encode(&completed).unwrap();
        assert_eq!(completed.revision().get(), 1);
        assert_eq!(
            StoredTaskRecord::from_storage_bytes(completed.clone().into_storage_bytes())
                .unwrap()
                .storage_bytes(),
            completed.storage_bytes()
        );

        assert!(
            StoredTaskRecord::from_storage_bytes(vec![b'x'; MAX_STORED_TASK_RECORD_BYTES + 1])
                .is_err()
        );
        let mut unsupported: Value = serde_json::from_slice(stored.storage_bytes()).unwrap();
        unsupported["version"] = json!(2);
        assert!(
            StoredTaskRecord::from_storage_bytes(serde_json::to_vec(&unsupported).unwrap())
                .is_err()
        );
    }

    #[test]
    fn storage_keys_bind_surface_access_task_and_scope() {
        let task_id = "33".repeat(32);
        let surface = "44".repeat(32);
        let capability =
            derive_task_storage_key(&surface, &TaskAccessPolicy::CapabilityId, &task_id, None)
                .unwrap();
        let scope = TaskAccessScope::new(b"principal-a").unwrap();
        let scoped = derive_task_storage_key(
            &surface,
            &TaskAccessPolicy::Scoped(Arc::new(FixedScopeProvider)),
            &task_id,
            Some(&scope),
        )
        .unwrap();
        assert_ne!(capability, scoped);
        assert_ne!(
            capability,
            derive_task_storage_key(
                &"55".repeat(32),
                &TaskAccessPolicy::CapabilityId,
                &task_id,
                None,
            )
            .unwrap()
        );
        assert_eq!(format!("{capability:?}"), "TaskStorageKey(<redacted>)");
    }

    #[tokio::test]
    async fn in_memory_admission_and_compare_and_set_are_atomic() {
        let store = InMemoryTaskStore::server_instance();
        let surface = "44".repeat(32);
        let mut first = None;
        for index in 0..MAX_STORED_TASKS {
            let task_id = format!("{index:064x}");
            let key =
                derive_task_storage_key(&surface, &TaskAccessPolicy::CapabilityId, &task_id, None)
                    .unwrap();
            let record = SemanticTaskRecord::working(
                task_id,
                surface.clone(),
                0,
                None,
                SemanticTaskProfile::TasksExtension,
                1_700_000_000_000,
                None,
            );
            let stored = StoredTaskRecord::encode(&record).unwrap();
            assert_eq!(
                store.create(key, stored.clone()).await.unwrap(),
                TaskStoreCreate::Created
            );
            if index == 0 {
                first = Some((key, record, stored));
            }
        }
        let extra_id = "ff".repeat(32);
        let extra_key =
            derive_task_storage_key(&surface, &TaskAccessPolicy::CapabilityId, &extra_id, None)
                .unwrap();
        let extra = StoredTaskRecord::encode(&SemanticTaskRecord::working(
            extra_id,
            surface.clone(),
            0,
            None,
            SemanticTaskProfile::TasksExtension,
            1_700_000_000_000,
            None,
        ))
        .unwrap();
        assert_eq!(
            store.create(extra_key, extra.clone()).await.unwrap(),
            TaskStoreCreate::CapacityExceeded
        );

        let (first_key, first_record, first_stored) = first.unwrap();
        assert_eq!(
            store.create(first_key, first_stored.clone()).await.unwrap(),
            TaskStoreCreate::Occupied
        );
        let completed = first_record.successor(
            SemanticTaskStatus::Completed,
            Some(json!({ "content": [], "resultType": "complete" })),
            1_700_000_000_001,
        );
        let completed = StoredTaskRecord::encode(&completed).unwrap();
        assert_eq!(
            store
                .compare_and_set(first_key, first_stored.revision(), completed.clone())
                .await
                .unwrap(),
            TaskStoreWrite::Written
        );
        assert_eq!(
            store
                .compare_and_set(first_key, first_stored.revision(), completed)
                .await
                .unwrap(),
            TaskStoreWrite::Conflict
        );
        assert_eq!(
            store.remove(first_key).await.unwrap(),
            TaskStoreRemoval::Removed
        );
        assert_eq!(
            store.create(extra_key, extra).await.unwrap(),
            TaskStoreCreate::Created
        );
    }

    struct FixedScopeProvider;

    impl TaskAccessScopeProvider for FixedScopeProvider {
        fn scope(
            &self,
            _context: TaskAccessContext<'_>,
        ) -> std::result::Result<TaskAccessScope, TaskAccessScopeError> {
            TaskAccessScope::new(b"principal-a")
        }
    }
}
