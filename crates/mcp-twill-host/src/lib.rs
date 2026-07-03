//! Runtime host for mcp-twill servers.
//!
//! The core framework constructs [`RuntimeIdentity`] from what a bare
//! registry knows: name, version, and contract hashes. This crate owns the
//! facts only a process-level host can supply — the process id and start
//! time — and the retry policy a supervisor needs when it considers
//! re-issuing a call.
//!
//! Hot-replacement detection (noticing the server binary was swapped under
//! a live connection) is planned follow-up work in this crate; nothing here
//! populates an executable hash or replacement status yet.

use chrono::Utc;
use mcp_twill::{CliMcpServer, EffectSpec, RuntimeIdentity};

/// Wraps a server with the identity facts a process-level host can observe.
///
/// Construction captures the start time, so build the host once at startup
/// and keep it for the life of the process.
#[derive(Debug, Clone)]
pub struct RuntimeHost {
    identity: RuntimeIdentity,
}

impl RuntimeHost {
    /// Captures process facts for the given server: process id and start
    /// time, layered onto the identity the server already reports.
    pub fn new(server: &CliMcpServer) -> Self {
        let identity = attach_process_facts(server.runtime_identity());
        Self { identity }
    }

    /// The hosted identity: everything the core server reports, plus the
    /// process id and start time.
    pub fn identity(&self) -> &RuntimeIdentity {
        &self.identity
    }

    /// Whether a call with this effect may be retried after an ambiguous
    /// failure (connection dropped mid-call, server replaced, timeout with
    /// unknown outcome).
    pub fn retry_decision(&self, effect: &EffectSpec, idempotency: Idempotency) -> RetryDecision {
        retry_decision(effect, idempotency)
    }
}

fn attach_process_facts(identity: RuntimeIdentity) -> RuntimeIdentity {
    let mut identity = identity;
    identity.process_id = Some(std::process::id());
    identity.started_at_unix_ms = Some(Utc::now().timestamp_millis());
    identity
}

/// Whether the handler declared an idempotency key for this invocation.
/// Retry policy trusts the declaration; verifying that a handler actually
/// deduplicates on the key is the handler author's contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// The handler declared an idempotency key for this call.
    Keyed,
    /// No idempotency declaration.
    None,
}

/// The host's answer to "may this call be re-issued after an ambiguous
/// failure?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Safe to retry: the effect observes but does not change anything.
    Retry,
    /// Safe to retry only because the handler declared an idempotency key.
    RetryWithKey,
    /// Not safe to retry; re-issuing could repeat the side effect.
    NoRetry {
        /// The effect that blocks the retry, for diagnostics.
        effect: EffectSpec,
    },
}

impl RetryDecision {
    pub fn is_retryable(&self) -> bool {
        !matches!(self, RetryDecision::NoRetry { .. })
    }
}

/// Effect-aware retry policy. Pure and read effects are always retryable.
/// Writes, deletes, process execution, and network calls are retryable only
/// when the handler declared an idempotency key; composites take the most
/// restrictive answer among their parts.
pub fn retry_decision(effect: &EffectSpec, idempotency: Idempotency) -> RetryDecision {
    match effect {
        EffectSpec::Pure | EffectSpec::Read => RetryDecision::Retry,
        EffectSpec::Composite(effects) => {
            let mut decision = RetryDecision::Retry;
            for part in effects {
                match retry_decision(part, idempotency) {
                    blocked @ RetryDecision::NoRetry { .. } => return blocked,
                    RetryDecision::RetryWithKey => decision = RetryDecision::RetryWithKey,
                    RetryDecision::Retry => {}
                }
            }
            decision
        }
        // Write, Delete, Exec, Network, and Custom effects all change
        // something outside the framework; ambiguous failure means the
        // change may already have happened.
        effect => match idempotency {
            Idempotency::Keyed => RetryDecision::RetryWithKey,
            Idempotency::None => RetryDecision::NoRetry {
                effect: effect.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_and_read_effects_retry_without_a_key() {
        assert_eq!(
            retry_decision(&EffectSpec::Pure, Idempotency::None),
            RetryDecision::Retry
        );
        assert_eq!(
            retry_decision(&EffectSpec::Read, Idempotency::None),
            RetryDecision::Retry
        );
    }

    #[test]
    fn side_effects_do_not_retry_without_a_key() {
        for effect in [
            EffectSpec::Write,
            EffectSpec::Delete,
            EffectSpec::Exec,
            EffectSpec::Network,
            EffectSpec::Custom("sync".to_string()),
        ] {
            let decision = retry_decision(&effect, Idempotency::None);
            assert!(!decision.is_retryable(), "{effect:?} must not retry");
            assert_eq!(decision, RetryDecision::NoRetry { effect });
        }
    }

    #[test]
    fn side_effects_retry_only_with_a_declared_key() {
        for effect in [EffectSpec::Write, EffectSpec::Delete, EffectSpec::Network] {
            assert_eq!(
                retry_decision(&effect, Idempotency::Keyed),
                RetryDecision::RetryWithKey
            );
        }
    }

    #[test]
    fn composites_take_the_most_restrictive_answer() {
        let read_write = EffectSpec::Composite(vec![EffectSpec::Read, EffectSpec::Write]);
        assert_eq!(
            retry_decision(&read_write, Idempotency::None),
            RetryDecision::NoRetry {
                effect: EffectSpec::Write
            }
        );
        assert_eq!(
            retry_decision(&read_write, Idempotency::Keyed),
            RetryDecision::RetryWithKey
        );

        let read_only = EffectSpec::Composite(vec![EffectSpec::Read, EffectSpec::Pure]);
        assert_eq!(
            retry_decision(&read_only, Idempotency::None),
            RetryDecision::Retry
        );
    }
}
