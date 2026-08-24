//! Bounded, redacted operational evidence.
//!
//! The flight recorder is deliberately in-memory and finite. Export uses a
//! write/sync/rename sequence so a crash cannot turn a previous report into a
//! partially written JSON document.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

pub const MIN_RETENTION: Duration = Duration::from_secs(30);
pub const MAX_RETENTION: Duration = Duration::from_secs(60);
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(45);
pub const DEFAULT_EVENT_CAPACITY: usize = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum OperationsError {
    #[error("flight recorder retention must be between 30 and 60 seconds")]
    InvalidRetention,
    #[error("flight recorder capacity must be non-zero")]
    InvalidCapacity,
    #[error("operations I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("operations JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, OperationsError>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub monotonic_nanos: u64,
    pub level: DiagnosticLevel,
    pub subsystem: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<u64>,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}

impl DiagnosticEvent {
    pub fn new(
        monotonic_nanos: u64,
        level: DiagnosticLevel,
        subsystem: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            sequence: 0,
            monotonic_nanos,
            level,
            subsystem: subsystem.into(),
            kind: kind.into(),
            frame_id: None,
            fields: BTreeMap::new(),
        }
    }

    pub fn frame(mut self, frame_id: u64) -> Self {
        self.frame_id = Some(frame_id);
        self
    }

    pub fn field(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }

    fn redacted(mut self) -> Self {
        self.subsystem = redact_text(&self.subsystem);
        self.kind = redact_text(&self.kind);
        self.fields = self
            .fields
            .into_iter()
            .map(|(key, value)| {
                let value = if sensitive_key(&key) {
                    Value::String("<redacted>".into())
                } else {
                    redact_value(value)
                };
                (key, value)
            })
            .collect();
        self
    }
}

#[derive(Clone, Debug)]
pub struct FlightRecorder {
    retention_nanos: u64,
    capacity: usize,
    next_sequence: u64,
    latest_monotonic_nanos: u64,
    events: VecDeque<DiagnosticEvent>,
}

impl Default for FlightRecorder {
    fn default() -> Self {
        Self::new(DEFAULT_RETENTION, DEFAULT_EVENT_CAPACITY)
            .expect("default recorder configuration is valid")
    }
}

impl FlightRecorder {
    pub fn new(retention: Duration, capacity: usize) -> Result<Self> {
        if !(MIN_RETENTION..=MAX_RETENTION).contains(&retention) {
            return Err(OperationsError::InvalidRetention);
        }
        if capacity == 0 {
            return Err(OperationsError::InvalidCapacity);
        }
        Ok(Self {
            retention_nanos: retention.as_nanos().min(u128::from(u64::MAX)) as u64,
            capacity,
            next_sequence: 1,
            latest_monotonic_nanos: 0,
            events: VecDeque::with_capacity(capacity.min(4096)),
        })
    }

    pub fn record(&mut self, mut event: DiagnosticEvent) {
        event = event.redacted();
        event.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.latest_monotonic_nanos = self.latest_monotonic_nanos.max(event.monotonic_nanos);
        self.events.push_back(event);
        self.prune();
    }

    pub fn snapshot(&self) -> Vec<DiagnosticEvent> {
        self.events.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn retention(&self) -> Duration {
        Duration::from_nanos(self.retention_nanos)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    fn prune(&mut self) {
        let oldest = self
            .latest_monotonic_nanos
            .saturating_sub(self.retention_nanos);
        while self
            .events
            .front()
            .is_some_and(|event| event.monotonic_nanos < oldest)
        {
            self.events.pop_front();
        }
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    Automated,
    HilPending,
    HilVerified,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityEntry {
    pub id: String,
    pub compiled: bool,
    pub available: bool,
    pub active: bool,
    pub detail: String,
    pub evidence: EvidenceState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityReport {
    pub schema_version: u32,
    pub generated_unix_millis: u64,
    pub product_version: String,
    pub target: String,
    pub no_implicit_fallback: bool,
    pub capabilities: Vec<CapabilityEntry>,
}

impl CapabilityReport {
    pub fn new(
        generated_unix_millis: u64,
        product_version: impl Into<String>,
        target: impl Into<String>,
        mut capabilities: Vec<CapabilityEntry>,
    ) -> Self {
        capabilities.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            schema_version: 1,
            generated_unix_millis,
            product_version: product_version.into(),
            target: target.into(),
            no_implicit_fallback: true,
            capabilities,
        }
    }

    pub fn export(&self, path: &Path) -> Result<()> {
        export_json_atomic(self, path)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CrashReport {
    pub schema_version: u32,
    pub generated_unix_millis: u64,
    pub reason: String,
    pub project_hash: String,
    pub diagnostics: Vec<DiagnosticEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilityReport>,
}

impl CrashReport {
    pub fn new(
        generated_unix_millis: u64,
        reason: impl Into<String>,
        project_hash: impl Into<String>,
        diagnostics: Vec<DiagnosticEvent>,
        capabilities: Option<CapabilityReport>,
    ) -> Self {
        Self {
            schema_version: 1,
            generated_unix_millis,
            reason: redact_text(&reason.into()),
            project_hash: project_hash.into(),
            diagnostics: diagnostics
                .into_iter()
                .map(DiagnosticEvent::redacted)
                .collect(),
            capabilities,
        }
    }

    pub fn export(&self, path: &Path) -> Result<()> {
        export_json_atomic(self, path)
    }
}

pub fn export_json_atomic<T: Serialize>(value: &T, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    {
        let mut file = File::create(&temporary)?;
        write_json(&mut file, &bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_json(writer: &mut dyn Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(&text)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(&key) {
                        Value::String("<redacted>".into())
                    } else {
                        redact_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "authorization",
        "cookie",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "native_handle",
        "raw_handle",
        "device_handle",
        "socket_handle",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive))
}

fn redact_text(text: &str) -> String {
    let mut output = text.to_owned();
    redact_after_prefix_case_insensitive(&mut output, "bearer ", &[' ', ',', ';']);
    for key in [
        "token",
        "access_token",
        "api_key",
        "apikey",
        "secret",
        "password",
        "authorization",
    ] {
        for delimiter in ["=", ":"] {
            redact_after_prefix_case_insensitive(
                &mut output,
                &format!("{key}{delimiter}"),
                &['&', ' ', ',', ';'],
            );
        }
    }
    output
}

fn redact_after_prefix_case_insensitive(output: &mut String, prefix: &str, terminators: &[char]) {
    let mut search_from = 0;
    loop {
        let lower = output.to_ascii_lowercase();
        let Some(relative) = lower[search_from..].find(prefix) else {
            break;
        };
        let value_start = search_from + relative + prefix.len();
        let value_end = output[value_start..]
            .find(|character| terminators.contains(&character))
            .map_or(output.len(), |offset| value_start + offset);
        if value_start == value_end || output[value_start..value_end] == "<redacted>" {
            search_from = value_end.max(value_start.saturating_add(1));
            if search_from >= output.len() {
                break;
            }
            continue;
        }
        output.replace_range(value_start..value_end, "<redacted>");
        search_from = value_start + "<redacted>".len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_is_bounded_by_time_and_capacity() {
        let mut recorder = FlightRecorder::new(Duration::from_secs(30), 3).unwrap();
        for second in [0, 10, 20, 31, 32] {
            recorder.record(DiagnosticEvent::new(
                second * 1_000_000_000,
                DiagnosticLevel::Info,
                "runtime",
                "tick",
            ));
        }
        let snapshot = recorder.snapshot();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].monotonic_nanos, 20_000_000_000);
        assert_eq!(snapshot[2].sequence, 5);
        assert!(FlightRecorder::new(Duration::from_secs(29), 1).is_err());
        assert!(FlightRecorder::new(Duration::from_secs(61), 1).is_err());
    }

    #[test]
    fn nested_tokens_and_native_handles_are_redacted() {
        let mut recorder = FlightRecorder::default();
        recorder.record(
            DiagnosticEvent::new(1, DiagnosticLevel::Error, "control", "request")
                .field("token", "top-secret")
                .field("native_handle", 0xdead_beefu64)
                .field(
                    "url",
                    "rtmp://host/live?token=query-secret&name=ok password=hunter2",
                )
                .field(
                    "nested",
                    serde_json::json!({"authorization": "Bearer nested-secret", "safe": 7}),
                ),
        );
        let json = serde_json::to_string(&recorder.snapshot()).unwrap();
        for secret in [
            "top-secret",
            "deadbeef",
            "query-secret",
            "hunter2",
            "nested-secret",
        ] {
            assert!(!json.contains(secret), "leaked {secret}: {json}");
        }
        assert!(json.contains("<redacted>"));
        assert!(json.contains("\"safe\":7"));
    }

    #[test]
    fn writer_errors_propagate() {
        struct DiskFull;
        impl Write for DiskFull {
            fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "injected disk full",
                ))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let error = write_json(&mut DiskFull, b"{}").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    }

    #[test]
    fn capability_export_is_atomic_and_truthful() {
        let root =
            std::env::temp_dir().join(format!("eiviz-capability-report-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("capabilities.json");
        let report = CapabilityReport::new(
            42,
            "0.1.0",
            "test-target",
            vec![CapabilityEntry {
                id: "decklink".into(),
                compiled: false,
                available: false,
                active: false,
                detail: "SDK not compiled; no fallback".into(),
                evidence: EvidenceState::HilPending,
            }],
        );
        report.export(&path).unwrap();
        let loaded: CapabilityReport = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded, report);
        assert!(loaded.no_implicit_fallback);
        assert!(!path.with_extension("tmp").exists());
        let _ = fs::remove_dir_all(root);
    }
}
