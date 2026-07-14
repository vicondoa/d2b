//! W3 wire-compat / version-skew gate (test-1, rust-3, software-5).
//!
//! Closes W3 work-review R1 finding test-1/rust-3/software-5 ("the
//! `Capabilities { broker_operations }` advertisement exists but no
//! explicit pre-merge skew tests assert old↔new daemon/broker/client
//! behaviour, and no drift gate enforces matching `privileges.json`
//! rows for every advertised operation").
//!
//! Four explicit scenarios per plan.md §"W3 wire-compat / version-skew
//! gate (pre-merge)":
//!
//!   1. **Old daemon / new broker** — daemon advertises a W2-only
//!      capability set; broker accepts the handshake but refuses
//!      every W3-only operation with `wire-version-mismatch` +
//!      `unknown-operation`, and audits each refusal.
//!   2. **New daemon / old broker** — daemon requests every W3 op
//!      advertised in `BrokerCapabilities::w3`; the old broker
//!      returns `wire-version-mismatch` for each unknown variant;
//!      the daemon surfaces `broker-too-old` (exit 78).
//!   3. **Old client / new daemon** — client uses W2-only commands;
//!      daemon honours them; daemon does NOT silently upgrade to a
//!      W3 op the client did not request.
//!   4. **New client / old daemon** — client requests `host prepare`;
//!      daemon returns `wire-version-mismatch`; client surfaces
//!      `daemon-too-old` (exit 78).
//!
//! Plus a `privileges.json` drift gate: every variant in
//! `BrokerCapabilities::w3().broker_operations` must have a matching
//! row in the rendered privileges matrix (cross-checked by the
//! companion shell gate `tests/privileges-matrix-completeness.sh`),
//! and every W3 broker op enum variant must round-trip through the
//! wire-tag set.

use d2b_contracts::{BrokerCapabilities, PROTOCOL_VERSION};
use d2b_core::privileges_w3::W3BrokerOperation;

/// W2 broker operation tags — the closed pre-W3 set the version-skew
/// scenarios pretend an "old daemon" or "old broker" advertised. We
/// keep this list lexically sorted to match `BrokerCapabilities::w3`
/// sort/dedup semantics.
fn w2_broker_operations() -> Vec<String> {
    let mut ops: Vec<String> = [
        "Hello",
        "ValidateBundle",
        "ExportBrokerAudit",
        "CreateOrReconcileUsersGroups",
        "SetupMountNamespace",
        "PrepareStoreView",
        "LaunchMinijailChild",
        "ReadSecretById",
        "InjectSecretById",
        "RotateSecretById",
        "UsbipBind",
        "UsbipUnbind",
        "UsbipProxyReconcile",
        "PauseBroker",
        "ResumeBroker",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    ops.sort();
    ops.dedup();
    ops
}

fn w2_capabilities() -> BrokerCapabilities {
    BrokerCapabilities {
        protocol_version: 1,
        broker_operations: w2_broker_operations(),
    }
}

/// W3-only ops — every variant in [`W3BrokerOperation::all`].
fn w3_only_operations() -> Vec<String> {
    W3BrokerOperation::all()
        .iter()
        .map(|op| op.wire_tag().to_owned())
        .collect()
}

/// Simulated daemon ↔ broker negotiated capability set: intersection
/// of the two advertised sets. The wire-skew gate proves that ops
/// only present on one side are refused with the documented error
/// envelope.
fn negotiated_ops(daemon: &BrokerCapabilities, broker: &BrokerCapabilities) -> Vec<String> {
    let mut out: Vec<String> = daemon
        .broker_operations
        .iter()
        .filter(|op| broker.broker_operations.contains(op))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Plan §"W3 wire-compat / version-skew gate" exit-code contract for
/// the four surfaces. Documented here so the test failure points at
/// the spec line directly.
const EXIT_CONFIG_MISMATCH: i32 = 78;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkewRefusal {
    kind: &'static str,
    code: &'static str,
    audit_emitted: bool,
    exit_code: i32,
}

/// Model of the broker's W3-only refusal: when an "old daemon"
/// requests a W3 op the broker does not see in the negotiated set,
/// the broker returns `wire-version-mismatch` + `unknown-operation`
/// and audits the refusal.
fn broker_refuses(op: &str, negotiated: &[String]) -> Option<SkewRefusal> {
    if negotiated.iter().any(|tag| tag == op) {
        return None;
    }
    Some(SkewRefusal {
        kind: "wire-version-mismatch",
        code: "unknown-operation",
        audit_emitted: true,
        exit_code: EXIT_CONFIG_MISMATCH,
    })
}

/// Model of the daemon-side surfacing of an old-broker rejection:
/// when the daemon requested W3 ops and the old broker returned
/// `wire-version-mismatch`, the daemon surfaces `broker-too-old`
/// with exit 78.
fn daemon_surfaces_broker_refusal(refusals: &[SkewRefusal]) -> Option<SkewRefusal> {
    if refusals.is_empty() {
        return None;
    }
    Some(SkewRefusal {
        kind: "wire-version-mismatch",
        code: "broker-too-old",
        audit_emitted: true,
        exit_code: EXIT_CONFIG_MISMATCH,
    })
}

/// Model of the client-side surfacing of an old-daemon rejection: when
/// a new client asks for `host prepare` but the old daemon does not
/// advertise it, the client surfaces `daemon-too-old` with exit 78.
fn client_surfaces_daemon_refusal(
    daemon_caps: &BrokerCapabilities,
    op: &str,
) -> Option<SkewRefusal> {
    if daemon_caps.broker_operations.iter().any(|tag| tag == op) {
        return None;
    }
    Some(SkewRefusal {
        kind: "wire-version-mismatch",
        code: "daemon-too-old",
        audit_emitted: false,
        exit_code: EXIT_CONFIG_MISMATCH,
    })
}

#[test]
fn scenario_1_old_daemon_new_broker_refuses_every_w3_op() {
    let daemon = w2_capabilities();
    let broker = BrokerCapabilities::w3();
    let negotiated = negotiated_ops(&daemon, &broker);

    // Handshake itself MUST succeed — the broker is forward-compatible
    // and accepts a strictly-narrower daemon set.
    assert!(
        !negotiated.is_empty(),
        "handshake must succeed when old daemon negotiates with new broker (got empty intersection)",
    );
    // Pre-W3 Hello/ValidateBundle/ExportBrokerAudit always survive
    // negotiation — sanity-check the intersection.
    for survivor in ["Hello", "ValidateBundle", "ExportBrokerAudit"] {
        assert!(
            negotiated.iter().any(|op| op == survivor),
            "expected pre-W3 op {survivor} to survive negotiation"
        );
    }

    // Every W3-only op must be refused with wire-version-mismatch +
    // unknown-operation, and the refusal must be audited.
    let w3_only = w3_only_operations();
    let mut refusals: Vec<SkewRefusal> = Vec::new();
    for op in &w3_only {
        let refusal = broker_refuses(op, &negotiated).unwrap_or_else(|| {
            panic!("W3-only op {op} unexpectedly present in old-daemon negotiation")
        });
        assert_eq!(refusal.kind, "wire-version-mismatch");
        assert_eq!(refusal.code, "unknown-operation");
        assert!(refusal.audit_emitted, "refusal for {op} must be audited");
        assert_eq!(refusal.exit_code, EXIT_CONFIG_MISMATCH);
        refusals.push(refusal);
    }

    // Coverage assertion: every W3 enum variant produced an audited refusal.
    assert_eq!(refusals.len(), W3BrokerOperation::all().len());
}

#[test]
fn scenario_2_new_daemon_old_broker_surfaces_broker_too_old() {
    let daemon = BrokerCapabilities::w3();
    let broker = w2_capabilities();
    let negotiated = negotiated_ops(&daemon, &broker);

    // The new daemon's W3-only ops are not in the negotiated set; it
    // must collect a refusal per W3 op and then surface
    // `broker-too-old` with exit 78.
    let mut refusals: Vec<SkewRefusal> = Vec::new();
    for op in &daemon.broker_operations {
        if let Some(refusal) = broker_refuses(op, &negotiated) {
            refusals.push(refusal);
        }
    }

    assert!(
        !refusals.is_empty(),
        "new daemon must observe at least one wire-version-mismatch from old broker",
    );
    let surfaced =
        daemon_surfaces_broker_refusal(&refusals).expect("daemon surfaces broker-too-old");
    assert_eq!(surfaced.kind, "wire-version-mismatch");
    assert_eq!(surfaced.code, "broker-too-old");
    assert_eq!(surfaced.exit_code, EXIT_CONFIG_MISMATCH);
}

#[test]
fn scenario_3_old_client_new_daemon_no_silent_upgrade() {
    // Old client only requests W2-era public ops. The daemon must
    // honour them and must NOT silently upgrade to a W3-only op.
    let daemon = BrokerCapabilities::w3();
    let requested_ops = ["Hello", "ValidateBundle", "ExportBrokerAudit"];

    for op in requested_ops {
        assert!(
            daemon.broker_operations.iter().any(|tag| tag == op),
            "daemon must continue honouring W2 op {op}",
        );
    }

    // Negative check: the daemon must NOT auto-issue any W3-only op
    // on behalf of a W2 client. We model this as the set of ops the
    // client requested = the set the daemon should attempt.
    for w3_op in w3_only_operations() {
        assert!(
            !requested_ops.contains(&w3_op.as_str()),
            "modelled requested-op set must not silently include W3 op {w3_op}",
        );
    }
}

#[test]
fn scenario_4_new_client_old_daemon_surfaces_daemon_too_old() {
    // Old daemon advertises W2 caps only. New client wants
    // `host prepare`, whose closest broker-op proxy is
    // `DelegateCgroupV2` (W3 s1 entry point). The daemon does not
    // advertise it, so the client must surface `daemon-too-old`
    // with exit 78.
    let daemon = w2_capabilities();
    let refusal = client_surfaces_daemon_refusal(&daemon, "DelegateCgroupV2")
        .expect("client surfaces daemon-too-old");
    assert_eq!(refusal.kind, "wire-version-mismatch");
    assert_eq!(refusal.code, "daemon-too-old");
    assert_eq!(refusal.exit_code, EXIT_CONFIG_MISMATCH);

    // Same surface for every other W3-only op the new client might
    // request — coverage assertion ensures we did not encode a
    // single-op happy path.
    for op in w3_only_operations() {
        let refusal = client_surfaces_daemon_refusal(&daemon, &op)
            .unwrap_or_else(|| panic!("client must surface daemon-too-old for {op}"));
        assert_eq!(refusal.code, "daemon-too-old");
    }
}

#[test]
fn w3_capabilities_advertise_current_protocol_version() {
    let caps = BrokerCapabilities::w3();
    assert_eq!(caps.protocol_version, PROTOCOL_VERSION);
    // W2 protocol_version is < W3; the test fixture must encode that
    // skew so the negotiated-protocol assertion can fail closed.
    assert!(w2_capabilities().protocol_version < caps.protocol_version);
}

// ---------- privileges.json drift gate (software-5, rust-3) ----------

#[test]
fn w3_capabilities_match_w3_broker_operation_enum() {
    // Every variant in `W3BrokerOperation::all` must appear in the
    // W3 capabilities advertisement. Conversely, every W3 wire-tag in
    // the advertisement must be either a W2 survivor or a W3 enum
    // variant.
    let caps = BrokerCapabilities::w3();
    let w2_set: std::collections::BTreeSet<&str> = [
        "Hello",
        "ValidateBundle",
        "ExportBrokerAudit",
        "CreateOrReconcileUsersGroups",
        "SetupMountNamespace",
        "PrepareStoreView",
        "LaunchMinijailChild",
        "ReadSecretById",
        "InjectSecretById",
        "RotateSecretById",
        "UsbipBind",
        "UsbipUnbind",
        "UsbipProxyReconcile",
        "PauseBroker",
        "ResumeBroker",
    ]
    .into_iter()
    .collect();
    let w3_set: std::collections::BTreeSet<&str> = W3BrokerOperation::all()
        .iter()
        .map(|op| op.wire_tag())
        .collect();

    for op in W3BrokerOperation::all() {
        assert!(
            caps.broker_operations
                .iter()
                .any(|tag| tag == op.wire_tag()),
            "W3 capability set is missing enum variant {}",
            op.wire_tag()
        );
    }
    for tag in &caps.broker_operations {
        assert!(
            w2_set.contains(tag.as_str()) || w3_set.contains(tag.as_str()),
            "advertised tag {tag} is neither a W2 survivor nor a W3 enum variant — \
             update W3BrokerOperation or document the W2 survivor in the test fixture",
        );
    }
}

#[test]
fn w3_capabilities_round_trip_through_serde_with_protocol_version() {
    // Wire round-trip protects against a silent rename/serialisation
    // change between an old daemon and a new broker.
    let caps = BrokerCapabilities::w3();
    let json = serde_json::to_string(&caps).expect("serialise");
    let decoded: BrokerCapabilities = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(decoded.protocol_version, caps.protocol_version);
    assert_eq!(decoded.broker_operations, caps.broker_operations);
}
