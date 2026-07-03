use std::sync::Mutex;

use chrono::Utc;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};

use crate::{Diagnostic, EffectSpec, InvocationPlan, ResponseEnvelope, ResponseStatus};

/// The framework's account of one tool call: what was planned, how
/// authorization went, and how dispatch ended. Events are not a substitute
/// for handler logs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkEvent {
    pub id: String,
    pub timestamp_unix_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    pub status: ResponseStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl FrameworkEvent {
    /// Builds the terminal event for a call from the response envelope and
    /// the invocation plan, when planning got far enough to produce one.
    pub fn from_envelope(envelope: &ResponseEnvelope, plan: Option<&InvocationPlan>) -> Self {
        let mut id_bytes = [0u8; 8];
        OsRng.fill_bytes(&mut id_bytes);
        Self {
            id: format!("event-{}", hex(&id_bytes)),
            timestamp_unix_ms: Utc::now().timestamp_millis(),
            operation_id: plan.map(|plan| plan.operation_id.clone()),
            command: envelope
                .command
                .clone()
                .or_else(|| plan.map(|plan| plan.command_path.clone())),
            status: envelope.status.clone(),
            effects: plan.map(|plan| vec![plan.effect.clone()]).unwrap_or_default(),
            diagnostics: envelope.diagnostics.clone(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Receives framework events. Implementations must be cheap and non-blocking;
/// the adapter records events inline on the request path.
pub trait EventSink: Send + Sync {
    fn record(&self, event: FrameworkEvent);
}

/// The default sink: discards every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn record(&self, _event: FrameworkEvent) {}
}

/// Buffers events in memory for tests and development inspection.
#[derive(Debug, Default)]
pub struct InMemoryEventSink {
    events: Mutex<Vec<FrameworkEvent>>,
}

impl InMemoryEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of every recorded event, oldest first.
    pub fn events(&self) -> Vec<FrameworkEvent> {
        self.events.lock().expect("event sink poisoned").clone()
    }

    pub fn is_empty(&self) -> bool {
        self.events.lock().expect("event sink poisoned").is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.lock().expect("event sink poisoned").len()
    }
}

impl EventSink for InMemoryEventSink {
    fn record(&self, event: FrameworkEvent) {
        self.events.lock().expect("event sink poisoned").push(event);
    }
}
