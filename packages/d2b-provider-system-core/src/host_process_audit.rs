//! Bounded ProcessEffect projections for Host child-process lifecycle events.
//!
//! `Provider/system-core` does not own the durable audit writer.  The fixed
//! Core effect adapter does.  This module therefore stops at a small typed
//! DTO and an injected port: it never imports the audit writer, accepts an
//! audit envelope, or carries a zone, path, PID, argv, or other host detail.

#![allow(dead_code)]

use serde::Serialize;
use std::fmt;

/// The closed Provider label for a process under a user-only Host.
pub const SYSTEM_CORE_USER_PROVIDER: &str = "system-core-user";
/// The closed Provider label for other Host process effects.
pub const SYSTEMD_PROVIDER: &str = "systemd";

/// The lifecycle event carried by a ProcessEffect record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessEffectEvent {
    /// A process was launched.
    Launch,
    /// A process was stopped.
    Stop,
    /// A previously running process was adopted.
    Adopt,
    /// A running process could not be safely adopted.
    Quarantine,
}

/// The execution domain carried by a ProcessEffect record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessEffectDomain {
    /// A system-domain process.
    System,
    /// A user-domain process.
    User,
}

/// The closed result carried by a ProcessEffect record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessEffectOutcome {
    /// The lifecycle operation completed.
    Ok,
    /// The lifecycle operation failed.
    Error,
}

/// The optional exit classification carried by a ProcessEffect record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessExitClass {
    /// The process exited normally.
    Exited,
    /// The process was terminated by a signal.
    Signaled,
    /// The process was forcibly killed.
    Killed,
}

/// A bounded, Core-adapter-ready ProcessEffect field projection.
///
/// The opaque digest and process identity are intentionally private so a
/// caller cannot construct an unvalidated record or accidentally add raw
/// identity to a diagnostic representation.  The Core adapter can serialize
/// this DTO into the authoritative audit envelope it owns.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProcessEffectFields {
    event: ProcessEffectEvent,
    provider: &'static str,
    domain: ProcessEffectDomain,
    no_isolation: bool,
    execution_ref_digest: String,
    process_uid: String,
    outcome: ProcessEffectOutcome,
    exit_class: Option<ProcessExitClass>,
}

impl fmt::Debug for ProcessEffectFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessEffectFields")
            .field("event", &self.event)
            .field("provider", &self.provider)
            .field("domain", &self.domain)
            .field("no_isolation", &self.no_isolation)
            .field("outcome", &self.outcome)
            .field("exit_class", &self.exit_class)
            .finish_non_exhaustive()
    }
}

impl ProcessEffectFields {
    /// Return the lifecycle event.
    pub const fn event(&self) -> ProcessEffectEvent {
        self.event
    }

    /// Return the closed Provider label.
    pub const fn provider(&self) -> &'static str {
        self.provider
    }

    /// Return the execution domain.
    pub const fn domain(&self) -> ProcessEffectDomain {
        self.domain
    }

    /// Whether the parent Host has no isolation.
    pub const fn no_isolation(&self) -> bool {
        self.no_isolation
    }

    /// Borrow the opaque execution-reference digest.
    pub fn execution_ref_digest(&self) -> &str {
        &self.execution_ref_digest
    }

    /// Borrow the opaque process identity.
    pub fn process_uid(&self) -> &str {
        &self.process_uid
    }

    /// Return the lifecycle outcome.
    pub const fn outcome(&self) -> ProcessEffectOutcome {
        self.outcome
    }

    /// Return the optional exit classification.
    pub const fn exit_class(&self) -> Option<ProcessExitClass> {
        self.exit_class
    }
}

/// A construction failure that carries no caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProcessAuditError {
    /// A user-only Host was asked to carry a system-domain process.
    UserOnlyHostRequiresUserDomain,
    /// The execution reference was not an opaque `sha256:` value.
    InvalidExecutionReferenceDigest,
    /// The process identity was empty, oversized, or contained unsafe text.
    InvalidProcessUid,
}

impl fmt::Display for HostProcessAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UserOnlyHostRequiresUserDomain => "user-only-host-requires-user-domain-process",
            Self::InvalidExecutionReferenceDigest => "process-effect-execution-digest-invalid",
            Self::InvalidProcessUid => "process-effect-process-uid-invalid",
        })
    }
}

impl std::error::Error for HostProcessAuditError {}

/// An injected Core-owned sink for bounded Host ProcessEffect fields.
///
/// The Provider emits only the typed fields.  Core supplies the implementation
/// that appends the fields to its durable audit envelope and decides the
/// standard-record durability policy.
pub trait HostProcessAuditPort {
    /// The sink-specific append error.
    type Error;

    /// Append one already-validated ProcessEffect projection.
    fn append_process_effect(&mut self, effect: ProcessEffectFields) -> Result<(), Self::Error>;
}

/// Failure while constructing or handing a ProcessEffect to Core.
#[derive(Debug, PartialEq, Eq)]
pub enum ProcessEffectEmitError<E> {
    /// The Provider refused to construct the effect.
    Invalid(HostProcessAuditError),
    /// Core's audit port refused the effect.
    Port(E),
}

/// Construct one validated ProcessEffect field projection.
pub fn process_effect_record(
    event: ProcessEffectEvent,
    domain: ProcessEffectDomain,
    user_only_host: bool,
    execution_ref_digest: impl Into<String>,
    process_uid: impl Into<String>,
    outcome: ProcessEffectOutcome,
    exit_class: Option<ProcessExitClass>,
) -> Result<ProcessEffectFields, HostProcessAuditError> {
    if user_only_host && domain != ProcessEffectDomain::User {
        return Err(HostProcessAuditError::UserOnlyHostRequiresUserDomain);
    }

    let execution_ref_digest = execution_ref_digest.into();
    if !valid_execution_digest(&execution_ref_digest) {
        return Err(HostProcessAuditError::InvalidExecutionReferenceDigest);
    }
    let process_uid = process_uid.into();
    if !valid_opaque_text(&process_uid) {
        return Err(HostProcessAuditError::InvalidProcessUid);
    }

    let (provider, no_isolation) = if user_only_host {
        (SYSTEM_CORE_USER_PROVIDER, true)
    } else {
        (SYSTEMD_PROVIDER, false)
    };
    Ok(ProcessEffectFields {
        event,
        provider,
        domain,
        no_isolation,
        execution_ref_digest,
        process_uid,
        outcome,
        exit_class,
    })
}

/// Emit the launch effect for a Host child process.
pub fn emit_launch<P: HostProcessAuditPort>(
    port: &mut P,
    domain: ProcessEffectDomain,
    user_only_host: bool,
    execution_ref_digest: impl Into<String>,
    process_uid: impl Into<String>,
    outcome: ProcessEffectOutcome,
) -> Result<(), ProcessEffectEmitError<P::Error>> {
    let effect = process_effect_record(
        ProcessEffectEvent::Launch,
        domain,
        user_only_host,
        execution_ref_digest,
        process_uid,
        outcome,
        None,
    )
    .map_err(ProcessEffectEmitError::Invalid)?;
    port.append_process_effect(effect)
        .map_err(ProcessEffectEmitError::Port)
}

/// Emit the stop effect for a Host child process.
pub fn emit_stop<P: HostProcessAuditPort>(
    port: &mut P,
    domain: ProcessEffectDomain,
    user_only_host: bool,
    execution_ref_digest: impl Into<String>,
    process_uid: impl Into<String>,
    outcome: ProcessEffectOutcome,
    exit_class: Option<ProcessExitClass>,
) -> Result<(), ProcessEffectEmitError<P::Error>> {
    let effect = process_effect_record(
        ProcessEffectEvent::Stop,
        domain,
        user_only_host,
        execution_ref_digest,
        process_uid,
        outcome,
        exit_class,
    )
    .map_err(ProcessEffectEmitError::Invalid)?;
    port.append_process_effect(effect)
        .map_err(ProcessEffectEmitError::Port)
}

fn valid_execution_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(valid_opaque_text)
}

fn valid_opaque_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::convert::Infallible;

    #[derive(Default)]
    struct RecordingPort {
        effects: Vec<ProcessEffectFields>,
        fail: bool,
    }

    impl HostProcessAuditPort for RecordingPort {
        type Error = Infallible;

        fn append_process_effect(
            &mut self,
            effect: ProcessEffectFields,
        ) -> Result<(), Self::Error> {
            if self.fail {
                return Ok(());
            }
            self.effects.push(effect);
            Ok(())
        }
    }

    #[test]
    fn user_only_launch_and_stop_carry_the_posture() {
        let launch = process_effect_record(
            ProcessEffectEvent::Launch,
            ProcessEffectDomain::User,
            true,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
            None,
        )
        .unwrap();
        assert_eq!(launch.event(), ProcessEffectEvent::Launch);
        assert_eq!(launch.provider(), SYSTEM_CORE_USER_PROVIDER);
        assert_eq!(launch.domain(), ProcessEffectDomain::User);
        assert!(launch.no_isolation());

        let stop = process_effect_record(
            ProcessEffectEvent::Stop,
            ProcessEffectDomain::User,
            true,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
            Some(ProcessExitClass::Exited),
        )
        .unwrap();
        assert_eq!(stop.event(), ProcessEffectEvent::Stop);
        assert_eq!(stop.exit_class(), Some(ProcessExitClass::Exited));
        assert!(stop.no_isolation());
    }

    #[test]
    fn non_user_only_effects_cannot_claim_the_posture() {
        let system = process_effect_record(
            ProcessEffectEvent::Launch,
            ProcessEffectDomain::System,
            false,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
            None,
        )
        .unwrap();
        assert_eq!(system.provider(), SYSTEMD_PROVIDER);
        assert_eq!(system.domain(), ProcessEffectDomain::System);
        assert!(!system.no_isolation());

        let user = process_effect_record(
            ProcessEffectEvent::Launch,
            ProcessEffectDomain::User,
            false,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
            None,
        )
        .unwrap();
        assert_eq!(user.provider(), SYSTEMD_PROVIDER);
        assert!(!user.no_isolation());
    }

    #[test]
    fn a_user_only_host_cannot_emit_a_system_process_effect() {
        assert_eq!(
            process_effect_record(
                ProcessEffectEvent::Launch,
                ProcessEffectDomain::System,
                true,
                "sha256:execution",
                "process",
                ProcessEffectOutcome::Ok,
                None,
            ),
            Err(HostProcessAuditError::UserOnlyHostRequiresUserDomain)
        );
    }

    #[test]
    fn malformed_opaque_inputs_are_rejected_before_the_port_is_called() {
        let mut port = RecordingPort::default();
        assert_eq!(
            emit_launch(
                &mut port,
                ProcessEffectDomain::User,
                true,
                "execution",
                "process",
                ProcessEffectOutcome::Ok,
            ),
            Err(ProcessEffectEmitError::Invalid(
                HostProcessAuditError::InvalidExecutionReferenceDigest
            ))
        );
        assert!(port.effects.is_empty());

        assert_eq!(
            process_effect_record(
                ProcessEffectEvent::Launch,
                ProcessEffectDomain::User,
                true,
                "sha256:execution",
                "/proc/secret",
                ProcessEffectOutcome::Ok,
                None,
            ),
            Err(HostProcessAuditError::InvalidProcessUid)
        );
    }

    #[test]
    fn launch_and_stop_call_sites_append_their_closed_events() {
        let mut port = RecordingPort::default();
        emit_launch(
            &mut port,
            ProcessEffectDomain::User,
            true,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
        )
        .unwrap();
        emit_stop(
            &mut port,
            ProcessEffectDomain::User,
            true,
            "sha256:execution",
            "process",
            ProcessEffectOutcome::Ok,
            Some(ProcessExitClass::Killed),
        )
        .unwrap();
        assert_eq!(port.effects.len(), 2);
        assert_eq!(port.effects[0].event(), ProcessEffectEvent::Launch);
        assert_eq!(port.effects[1].event(), ProcessEffectEvent::Stop);
        assert!(port.effects.iter().all(ProcessEffectFields::no_isolation));
    }

    #[test]
    fn debug_redacts_opaque_effect_values_and_serialization_is_bounded() {
        let canary = "process-identity-canary";
        let effect = process_effect_record(
            ProcessEffectEvent::Launch,
            ProcessEffectDomain::User,
            true,
            format!("sha256:{canary}"),
            canary,
            ProcessEffectOutcome::Ok,
            None,
        )
        .unwrap();
        let debug = format!("{effect:?}");
        assert!(!debug.contains(canary));
        assert!(!debug.contains("execution_ref_digest"));
        assert!(!debug.contains("process_uid"));

        let value = serde_json::to_value(effect).unwrap();
        assert_eq!(value["event"], "launch");
        assert_eq!(value["provider"], SYSTEM_CORE_USER_PROVIDER);
        assert_eq!(value["domain"], "user");
        assert_eq!(value["no_isolation"], true);
        assert_eq!(value["execution_ref_digest"], format!("sha256:{canary}"));
        assert_eq!(value["process_uid"], canary);
        assert_eq!(value["outcome"], "ok");
        assert!(value["exit_class"].is_null());
        let keys: BTreeSet<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "domain".to_owned(),
                "event".to_owned(),
                "execution_ref_digest".to_owned(),
                "exit_class".to_owned(),
                "no_isolation".to_owned(),
                "outcome".to_owned(),
                "process_uid".to_owned(),
                "provider".to_owned(),
            ])
        );
    }
}
