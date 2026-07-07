use std::sync::Mutex;

use chrono::Utc;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};

use crate::{
    Diagnostic, EffectSpec, InvocationPlan, ResponseEnvelope, ResponseStatus, RuntimeIdentity,
};

/// The framework's account of one tool call: what was planned, how
/// authorization went, and how dispatch ended. Events are not a substitute
/// for handler logs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkEvent {
    pub id: String,
    pub timestamp_unix_ms: i64,
    /// The identity of the server instance that recorded this event, when
    /// the adapter knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeIdentity>,
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
    /// the plan facts, when planning got far enough to produce them.
    pub fn from_envelope(envelope: &ResponseEnvelope, plan: Option<&PlanFacts>) -> Self {
        Self {
            id: new_event_id(),
            timestamp_unix_ms: Utc::now().timestamp_millis(),
            runtime: None,
            operation_id: plan.map(|plan| plan.operation_id.clone()),
            command: envelope
                .command
                .clone()
                .or_else(|| plan.map(|plan| plan.command_path.clone())),
            status: envelope.status.clone(),
            effects: plan
                .map(|plan| vec![plan.effect.clone()])
                .unwrap_or_default(),
            diagnostics: envelope.diagnostics.clone(),
        }
    }

    /// Builds the event for a call whose arguments never parsed as a run
    /// request, so no plan or envelope exists.
    pub fn parse_failure(message: impl Into<String>) -> Self {
        Self {
            id: new_event_id(),
            timestamp_unix_ms: Utc::now().timestamp_millis(),
            runtime: None,
            operation_id: None,
            command: None,
            status: ResponseStatus::InvalidInput,
            effects: Vec::new(),
            diagnostics: vec![Diagnostic {
                code: crate::ErrorCode::InvalidArgumentType,
                message: message.into(),
                location: None,
                expected: None,
                actual: None,
                suggestions: Vec::new(),
            }],
        }
    }

    /// Attaches the identity of the server instance recording the event.
    pub fn with_runtime(mut self, runtime: RuntimeIdentity) -> Self {
        self.runtime = Some(runtime);
        self
    }
}

/// The slice of an invocation plan that events need, extracted so the flow
/// does not clone the full plan per call.
#[derive(Debug, Clone)]
pub struct PlanFacts {
    pub operation_id: String,
    pub command_path: Vec<String>,
    pub effect: EffectSpec,
}

impl From<&InvocationPlan> for PlanFacts {
    fn from(plan: &InvocationPlan) -> Self {
        Self {
            operation_id: plan.operation_id.clone(),
            command_path: plan.command_path.clone(),
            effect: plan.effect.clone(),
        }
    }
}

fn new_event_id() -> String {
    let mut id_bytes = [0u8; 8];
    OsRng.fill_bytes(&mut id_bytes);
    format!("event-{}", hex(&id_bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Receives framework events. The adapter records events inline on the
/// request path, so implementations should return quickly and avoid
/// long-blocking work; brief locking (as in [`InMemoryEventSink`]) is fine.
pub trait EventSink: Send + Sync {
    fn record(&self, event: FrameworkEvent);

    /// Whether this sink wants events at all. The adapter skips event
    /// construction entirely when this returns false, so disabled sinks
    /// cost nothing per call.
    fn enabled(&self) -> bool {
        true
    }
}

/// The default sink: discards every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn record(&self, _event: FrameworkEvent) {}

    fn enabled(&self) -> bool {
        false
    }
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
