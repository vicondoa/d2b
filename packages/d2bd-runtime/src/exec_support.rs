//! Provider-neutral exec error, metric, and request projections.

use d2b_contracts_control::public_wire;

use crate::exec_session;
use crate::metrics::Registry;
use crate::typed_error::{GuestControlExecErrorKind, TypedError};

pub const EXEC_METRIC: &str = "d2b_daemon_guest_control_exec_total";
pub const EXEC_SUBSYSTEM: &str = "guest-control-exec";
pub const EXEC_OUTCOME_LABELS: &[&str] = &["established", "closed", "error", "op-error"];
pub const EXEC_ERROR_KIND_LABELS: &[&str] = &[
    "none",
    "transport",
    "auth",
    "protocol",
    "timeout",
    "old-generation",
    "capability",
    "detached-unavailable",
    "session-capacity",
    "rate-limited",
    "stale-session",
    "exec-not-found",
    "exec-expired",
    "invalid-program",
    "guest",
    "internal",
    "inflight-cap-exceeded",
];

pub fn exec_metric_into(registry: &Registry, outcome: &'static str, error_kind: &'static str) {
    debug_assert!(EXEC_OUTCOME_LABELS.contains(&outcome));
    debug_assert!(EXEC_ERROR_KIND_LABELS.contains(&error_kind));
    registry.counter_inc(
        EXEC_METRIC,
        &[
            ("subsystem", EXEC_SUBSYSTEM),
            ("outcome", outcome),
            ("error_kind", error_kind),
        ],
    );
}

pub fn exec_op_session(op: &public_wire::ExecOp) -> Option<&str> {
    match op {
        public_wire::ExecOp::Start(_) => None,
        public_wire::ExecOp::WriteStdin(args) => Some(&args.session),
        public_wire::ExecOp::ReadOutput(args) => Some(&args.session),
        public_wire::ExecOp::Signal(args) => Some(&args.session),
        public_wire::ExecOp::Resize(args) => Some(&args.session),
        public_wire::ExecOp::Wait(args) => Some(&args.session),
        public_wire::ExecOp::Close(args) => Some(&args.session),
        public_wire::ExecOp::List(_)
        | public_wire::ExecOp::Logs(_)
        | public_wire::ExecOp::Status(_)
        | public_wire::ExecOp::Kill(_) => None,
    }
}

pub fn map_exec_establish_error(error: exec_session::ExecEstablishError) -> TypedError {
    use exec_session::ExecEstablishError as Error;
    let kind = match error {
        Error::Transport => GuestControlExecErrorKind::Transport,
        Error::Auth => GuestControlExecErrorKind::Auth,
        Error::Protocol => GuestControlExecErrorKind::Protocol,
        Error::Timeout => GuestControlExecErrorKind::Timeout,
        Error::OldGeneration => GuestControlExecErrorKind::OldGeneration,
        Error::Capability => GuestControlExecErrorKind::Capability,
        Error::Guest(inner) => map_guest_exec_error_kind(inner),
    };
    TypedError::GuestControlExecFailed { kind }
}

pub fn map_guest_exec_error_kind(error: exec_session::GuestOpError) -> GuestControlExecErrorKind {
    use exec_session::GuestOpError as Error;
    match error {
        Error::ExecNotFound => GuestControlExecErrorKind::ExecNotFound,
        Error::ExecExpired => GuestControlExecErrorKind::ExecExpired,
        Error::InvalidProgram => GuestControlExecErrorKind::InvalidProgram,
        Error::Protocol => GuestControlExecErrorKind::Protocol,
        _ => GuestControlExecErrorKind::GuestError,
    }
}

pub fn map_exec_op_error(error: exec_session::ExecOpError) -> TypedError {
    use exec_session::ExecOpError as Error;
    let kind = match error {
        Error::Transport => GuestControlExecErrorKind::Transport,
        Error::Auth => GuestControlExecErrorKind::Auth,
        Error::StaleSession => GuestControlExecErrorKind::StaleSession,
        Error::Protocol => GuestControlExecErrorKind::Protocol,
        Error::Timeout => GuestControlExecErrorKind::Timeout,
        Error::OldGeneration => GuestControlExecErrorKind::OldGeneration,
        Error::Capability => GuestControlExecErrorKind::Capability,
        Error::DetachedUnavailable => GuestControlExecErrorKind::DetachedUnavailable,
        Error::Guest(inner) => map_guest_exec_error_kind(inner),
    };
    TypedError::GuestControlExecFailed { kind }
}

pub fn map_exec_reserve_error(error: exec_session::SessionReserveError) -> TypedError {
    use exec_session::SessionReserveError as Error;
    let kind = match error {
        Error::RateLimited => GuestControlExecErrorKind::RateLimited,
        _ => GuestControlExecErrorKind::SessionCapacity,
    };
    TypedError::GuestControlExecFailed { kind }
}

pub fn exec_error_kind_label(error: &TypedError) -> &'static str {
    match error {
        TypedError::GuestControlExecFailed { kind } => match kind {
            GuestControlExecErrorKind::Transport => "transport",
            GuestControlExecErrorKind::Auth => "auth",
            GuestControlExecErrorKind::Protocol => "protocol",
            GuestControlExecErrorKind::Timeout => "timeout",
            GuestControlExecErrorKind::OldGeneration => "old-generation",
            GuestControlExecErrorKind::Capability => "capability",
            GuestControlExecErrorKind::DetachedUnavailable => "detached-unavailable",
            GuestControlExecErrorKind::SessionCapacity => "session-capacity",
            GuestControlExecErrorKind::RateLimited => "rate-limited",
            GuestControlExecErrorKind::StaleSession => "stale-session",
            GuestControlExecErrorKind::ExecNotFound => "exec-not-found",
            GuestControlExecErrorKind::ExecExpired => "exec-expired",
            GuestControlExecErrorKind::InvalidProgram => "invalid-program",
            GuestControlExecErrorKind::GuestError => "guest",
            GuestControlExecErrorKind::Internal => "internal",
        },
        _ => "internal",
    }
}

pub fn emit_exec_established_event(vm: &str, peer_uid: u32, tty: bool) {
    tracing::info!(
        kind = "critical",
        subsystem = EXEC_SUBSYSTEM,
        vm = %vm,
        peer_uid = peer_uid,
        tty = tty,
        "guest-control exec session established"
    );
}

#[cfg(test)]
mod exec_metric_tests {
    //! The exec metric `d2b_daemon_guest_control_exec_total` is
    //! a HARD closed-label series. Its only labels are the constant
    //! `subsystem` plus a bounded `outcome` / `error_kind` enum - never a vm
    //! name, session handle, op id, peer uid, or argv hash. These tests assert
    //! the descriptor shape, the closed value sets, and that a rendered series
    //! carries nothing else.

    use super::{
        EXEC_ERROR_KIND_LABELS, EXEC_METRIC, EXEC_OUTCOME_LABELS, EXEC_SUBSYSTEM,
        exec_error_kind_label, exec_metric_into,
    };
    use crate::typed_error::{GuestControlExecErrorKind, TypedError};

    /// Every `GuestControlExecErrorKind` the daemon can surface (closed enum).
    const ALL_EXEC_ERROR_KINDS: &[GuestControlExecErrorKind] = &[
        GuestControlExecErrorKind::Transport,
        GuestControlExecErrorKind::Auth,
        GuestControlExecErrorKind::Protocol,
        GuestControlExecErrorKind::Timeout,
        GuestControlExecErrorKind::OldGeneration,
        GuestControlExecErrorKind::Capability,
        GuestControlExecErrorKind::DetachedUnavailable,
        GuestControlExecErrorKind::SessionCapacity,
        GuestControlExecErrorKind::RateLimited,
        GuestControlExecErrorKind::StaleSession,
        GuestControlExecErrorKind::ExecNotFound,
        GuestControlExecErrorKind::ExecExpired,
        GuestControlExecErrorKind::InvalidProgram,
        GuestControlExecErrorKind::GuestError,
        GuestControlExecErrorKind::Internal,
    ];

    #[test]
    fn exec_metric_descriptor_has_only_three_closed_labels() {
        // The inventory descriptor for the exec metric must declare EXACTLY
        // the three closed keys - adding `vm`, `session`, `op_id`, or any
        // per-session identifier here is the regression this guards.
        let descriptor =
            crate::metrics::descriptor(EXEC_METRIC).expect("exec metric is in the inventory");
        assert_eq!(
            descriptor.labels,
            &["subsystem", "outcome", "error_kind"],
            "exec metric must carry only the closed subsystem/outcome/error_kind labels"
        );
    }

    #[test]
    fn exec_error_kind_label_is_within_closed_allowlist() {
        // Every typed exec error maps to a label inside the closed set, so the
        // `error_kind` cardinality can never exceed the enum.
        for kind in ALL_EXEC_ERROR_KINDS {
            let error = TypedError::GuestControlExecFailed { kind: *kind };
            let label = exec_error_kind_label(&error);
            assert!(
                EXEC_ERROR_KIND_LABELS.contains(&label),
                "exec_error_kind_label returned {label:?} which is outside the closed allowlist"
            );
        }
        // A non-exec TypedError defaults to the `internal` bucket (still closed).
        assert_eq!(
            exec_error_kind_label(&TypedError::AuthzNotAdmin {
                verb: "exec".to_owned()
            }),
            "internal"
        );
    }

    #[test]
    fn exec_metric_labels_are_closed_enum() {
        // Emit one sample for EVERY (outcome, error_kind) pair in the closed
        // sets, render, and assert the rendered exec series carries only the
        // three approved keys, the constant subsystem, and closed values -
        // and never a forbidden per-session identifier.
        let registry = crate::metrics::Registry::new();
        for &outcome in EXEC_OUTCOME_LABELS {
            for &error_kind in EXEC_ERROR_KIND_LABELS {
                exec_metric_into(&registry, outcome, error_kind);
            }
        }
        let body = registry.render();

        let mut saw_exec_series = false;
        for line in body.lines() {
            if !line.starts_with(EXEC_METRIC) {
                continue;
            }
            let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else {
                continue;
            };
            saw_exec_series = true;
            let inner = &line[open + 1..close];
            for pair in inner.split(',') {
                let mut kv = pair.splitn(2, '=');
                let key = kv.next().unwrap_or("").trim();
                let value = kv.next().unwrap_or("").trim().trim_matches('"');
                match key {
                    "subsystem" => assert_eq!(
                        value, EXEC_SUBSYSTEM,
                        "exec subsystem label must be the constant guest-control-exec"
                    ),
                    "outcome" => assert!(
                        EXEC_OUTCOME_LABELS.contains(&value),
                        "exec outcome label {value:?} is outside the closed allowlist"
                    ),
                    "error_kind" => assert!(
                        EXEC_ERROR_KIND_LABELS.contains(&value),
                        "exec error_kind label {value:?} is outside the closed allowlist"
                    ),
                    other => panic!("exec metric leaked an unapproved label key {other:?}: {line}"),
                }
            }
        }
        assert!(
            saw_exec_series,
            "expected the exec metric to render a series"
        );

        // Belt-and-suspenders: no per-session identifier may ever appear as a
        // label key on the exec metric.
        for forbidden in [
            "vm=\"",
            "session=\"",
            "handle=\"",
            "op_id=\"",
            "op-id=\"",
            "peer_uid=\"",
            "uid=\"",
            "argv=\"",
            "argv_hash=\"",
        ] {
            for line in body.lines().filter(|l| l.starts_with(EXEC_METRIC)) {
                assert!(
                    !line.contains(forbidden),
                    "exec metric leaked forbidden label {forbidden:?}: {line}"
                );
            }
        }
    }
}

#[cfg(test)]
mod exec_established_tracing_tests {
    //! The single kind=critical exec session-establishment event must carry
    //! ONLY redaction-safe identifiers. This guards against a future edit that
    //! adds argv/env/cwd/output bytes (or any guest-supplied string) to the
    //! span, which would leak operator command lines into the daemon log.

    use super::emit_exec_established_event;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    type CapturedEvents = Arc<Mutex<Vec<Vec<(String, String)>>>>;

    #[derive(Default)]
    struct FieldCollector {
        fields: Vec<(String, String)>,
    }
    impl Visit for FieldCollector {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    struct CapturingLayer {
        events: CapturedEvents,
    }
    impl<S: tracing::Subscriber> Layer<S> for CapturingLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut collector = FieldCollector::default();
            event.record(&mut collector);
            self.events.lock().unwrap().push(collector.fields);
        }
    }

    #[test]
    fn exec_established_event_carries_only_leak_safe_fields() {
        // APPROVED field-name allowlist for the establishment event. The opaque
        // session handle is deliberately NOT approved - per AGENTS, session
        // handles must never reach a span, log, audit, or metric.
        const APPROVED_FIELDS: &[&str] = &["message", "kind", "subsystem", "vm", "peer_uid", "tty"];
        // Field names that MUST NEVER appear (would leak the command line, the
        // session handle, or guest-supplied content).
        const FORBIDDEN_FIELDS: &[&str] = &[
            "argv",
            "command",
            "cmd",
            "env",
            "cwd",
            "stdin",
            "stdout",
            "stderr",
            "output",
            "nonce",
            "token",
            "auth_tag",
            "exec_id",
            "guest_boot_id",
            "session_handle",
            "session",
            "handle",
        ];
        // A sentinel that would only appear if argv/env/cwd leaked into a field.
        const SENTINEL: &str = "D2B_ARGV_LEAK_CANARY";

        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CapturingLayer {
            events: events.clone(),
        };
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            emit_exec_established_event("work", 1000, true);
        });

        let captured = events.lock().unwrap();
        assert_eq!(
            captured.len(),
            1,
            "expected exactly one establishment event"
        );
        for fields in captured.iter() {
            assert!(!fields.is_empty(), "establishment event recorded no fields");
            for (name, value) in fields {
                assert!(
                    APPROVED_FIELDS.contains(&name.as_str()),
                    "unapproved establishment tracing field: {name}={value}"
                );
                assert!(
                    !FORBIDDEN_FIELDS.contains(&name.as_str()),
                    "forbidden establishment tracing field: {name}"
                );
                assert!(
                    !value.contains(SENTINEL),
                    "argv/env/cwd sentinel leaked: {name}={value}"
                );
            }
        }
    }
}
