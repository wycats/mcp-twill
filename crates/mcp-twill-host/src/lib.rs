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
use mcp_twill::{CliMcpServer, EffectSpec, InvocationPlan, RuntimeIdentity};

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

    /// Whether the planned call may be re-issued after an ambiguous failure
    /// (connection dropped mid-call, server replaced, timeout with unknown
    /// outcome). Reads the effect and the idempotency declaration from the
    /// plan — the catalog is authoritative, not the supervisor's judgment.
    pub fn retry_decision(&self, plan: &InvocationPlan) -> RetryDecision {
        retry_decision(plan)
    }
}

fn attach_process_facts(identity: RuntimeIdentity) -> RuntimeIdentity {
    let mut identity = identity;
    identity.process_id = Some(std::process::id());
    identity.started_at_unix_ms = Some(Utc::now().timestamp_millis());
    identity
}

/// The host's answer to "may this call be re-issued after an ambiguous
/// failure?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Safe to retry: the effect observes but does not change anything.
    Retry,
    /// Safe to retry only because the command declared itself idempotent
    /// in the catalog; the handler deduplicates re-issued invocations
    /// (the plan's invocation fingerprint is the natural key).
    RetryAsIdempotent,
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

/// Effect-aware retry policy, reading both the effect and the idempotency
/// declaration from the plan. Pure and read effects are always retryable.
/// Writes, deletes, process execution, and network calls are retryable only
/// when the command declared itself idempotent in the catalog; composites
/// take the most restrictive answer among their parts.
///
/// The declaration is trusted, like every other catalog fact; verifying
/// that a handler actually deduplicates is the handler author's contract.
pub fn retry_decision(plan: &InvocationPlan) -> RetryDecision {
    effect_retry(&plan.effect, plan.idempotent)
}

fn effect_retry(effect: &EffectSpec, idempotent: bool) -> RetryDecision {
    match effect {
        EffectSpec::Pure | EffectSpec::Read => RetryDecision::Retry,
        EffectSpec::Composite(effects) => {
            let mut decision = RetryDecision::Retry;
            for part in effects {
                match effect_retry(part, idempotent) {
                    blocked @ RetryDecision::NoRetry { .. } => return blocked,
                    RetryDecision::RetryAsIdempotent => {
                        decision = RetryDecision::RetryAsIdempotent;
                    }
                    RetryDecision::Retry => {}
                }
            }
            decision
        }
        // Write, Delete, Exec, Network, and Custom effects all change
        // something outside the framework; ambiguous failure means the
        // change may already have happened.
        effect => {
            if idempotent {
                RetryDecision::RetryAsIdempotent
            } else {
                RetryDecision::NoRetry {
                    effect: effect.clone(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_and_read_effects_retry_without_a_declaration() {
        assert_eq!(effect_retry(&EffectSpec::Pure, false), RetryDecision::Retry);
        assert_eq!(effect_retry(&EffectSpec::Read, false), RetryDecision::Retry);
    }

    #[test]
    fn side_effects_do_not_retry_without_a_declaration() {
        for effect in [
            EffectSpec::Write,
            EffectSpec::Delete,
            EffectSpec::Exec,
            EffectSpec::Network,
            EffectSpec::Custom("sync".to_string()),
        ] {
            let decision = effect_retry(&effect, false);
            assert!(!decision.is_retryable(), "{effect:?} must not retry");
            assert_eq!(decision, RetryDecision::NoRetry { effect });
        }
    }

    #[test]
    fn side_effects_retry_only_when_declared_idempotent() {
        for effect in [EffectSpec::Write, EffectSpec::Delete, EffectSpec::Network] {
            assert_eq!(
                effect_retry(&effect, true),
                RetryDecision::RetryAsIdempotent
            );
        }
    }

    #[test]
    fn composites_take_the_most_restrictive_answer() {
        let read_write = EffectSpec::Composite(vec![EffectSpec::Read, EffectSpec::Write]);
        assert_eq!(
            effect_retry(&read_write, false),
            RetryDecision::NoRetry {
                effect: EffectSpec::Write
            }
        );
        assert_eq!(
            effect_retry(&read_write, true),
            RetryDecision::RetryAsIdempotent
        );

        let read_only = EffectSpec::Composite(vec![EffectSpec::Read, EffectSpec::Pure]);
        assert_eq!(effect_retry(&read_only, false), RetryDecision::Retry);
    }
}
