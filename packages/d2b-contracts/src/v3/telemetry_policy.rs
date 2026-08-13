//! Canonical closed telemetry label and resource-attribute policy data.
//!
//! The emitter and the observability Provider own different validators, but
//! they must admit the same closed data domains. Keep the policy data here so
//! neither side can silently grow a divergent fork.

/// Exact keys which can never be metric dimensions.
pub const FORBIDDEN_LABEL_KEYS: &[&str] = &[
    "vm",
    "zone",
    "zone_id",
    "zone_uid",
    "credential",
    "credential_name",
    "network",
    "network_name",
    "guest",
    "host",
    "user",
    "volume",
    "device",
    "process",
    "link_name_hash",
];

/// Suffixes which indicate a resource identity dimension.
pub const FORBIDDEN_LABEL_SUFFIXES: &[&str] = &["_name", "_name_hash", "_name_digest", "_uid"];

/// Resource attributes allowed on OTEL records.
pub const OTEL_RESOURCE_ATTRIBUTES: &[&str] = &[
    "deployment.environment",
    "host.name",
    "service.name",
    "service.namespace",
    "source",
    "vm.env",
    "vm.name",
    "vm.role",
    "d2b.zone",
    "d2b.provider",
    "d2b.component",
    "service.version",
];

const RESOURCE_TYPE_VALUES: &[&str] = &[
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "Host",
    "Guest",
    "Process",
    "EphemeralProcess",
    "Volume",
    "Network",
    "Device",
    "User",
    "Credential",
    "Endpoint",
    "ResourceExport",
    "ResourceImport",
    "vendor",
];

/// Closed value domains for accepted metric-label keys.
pub const METRIC_LABEL_POLICY: &[(&str, &[&str])] = &[
    (
        "service",
        &[
            "resource-api",
            "bus",
            "session",
            "store",
            "controller",
            "d2b.resource.v3",
            "d2b.controller.v3",
            "d2b.provider.v3",
            "d2b.audit.v3",
            "d2b.support.v3",
            "d2b.credential.v3",
            "d2b.zone.v3",
            "d2b.zonelink.v3",
            "d2b.volume.v3",
        ],
    ),
    (
        "direction",
        &["local", "host", "guest", "zone_link", "send", "recv"],
    ),
    (
        "outcome",
        &[
            "ok",
            "error",
            "denied",
            "not_found",
            "conflict",
            "invalid",
            "quota",
            "auth",
            "transcript",
            "policy",
            "timeout",
            "requeue",
            "rate_limited",
            "abandoned",
            "dropped",
            "buffering",
            "accepted",
            "rejected",
            "quarantined",
            "cancel",
            "revoked",
            "degraded",
            "requested",
            "committed",
            "already-committed",
            "failed",
            "management",
            "established",
            "closed",
        ],
    ),
    ("resource_type", RESOURCE_TYPE_VALUES),
    ("exported_type", RESOURCE_TYPE_VALUES),
    ("projection_type", RESOURCE_TYPE_VALUES),
    (
        "verb",
        &[
            "get",
            "list",
            "watch",
            "create",
            "update-spec",
            "update-status",
            "update-metadata",
            "update-finalizers",
            "delete",
            "use-credential",
            "admin-credential",
        ],
    ),
    ("profile", &["NN", "KK", "IKpsk2"]),
    ("purpose_class", &["local", "enrolled", "bootstrap"]),
    ("transport", &["unix", "vsock", "zone_link"]),
    (
        "kind",
        &[
            "control",
            "ttrpc",
            "stream",
            "attachment",
            "single",
            "group",
        ],
    ),
    (
        "handler",
        &[
            "configuration",
            "api_catalog",
            "authz",
            "provider",
            "controller_registration",
            "ownership",
            "watch_maintenance",
            "ephemeral_cleanup",
            "zone_link",
            "budget",
            "store_lifecycle",
            "system_core_host",
            "system_core_user",
        ],
    ),
    (
        "provider",
        &[
            "minijail",
            "systemd",
            "system-core-user",
            "system-core",
            "observability-otel",
            "provider-managed",
            "local-vm",
            "qemu-media",
            "unsafe-local",
            "guest-control",
        ],
    ),
    ("domain", &["system", "user"]),
    (
        "operation",
        &[
            "get",
            "list",
            "scan",
            "create",
            "update",
            "delete",
            "admit",
            "revoke",
            "reconnect",
            "write",
            "read",
            "cancel",
            "launcher-exec",
        ],
    ),
    (
        "op",
        &[
            "Hello",
            "ValidateBundle",
            "ExportBrokerAudit",
            "ApplyNftables",
            "ApplyRoute",
            "ApplySysctl",
            "StoreSync",
            "StoreVerify",
            "SpawnRunner",
            "SignalRunner",
            "OpenPidfd",
            "CreateBridge",
            "CreatePersistentTap",
            "UsbipBind",
            "UsbipUnbind",
            "QemuMediaBoot",
            "QemuMediaAttach",
            "QemuMediaDetach",
            "vmStart",
            "vmStop",
            "vmRestart",
        ],
    ),
    (
        "record_class",
        &[
            "resource-mutation",
            "resource-upgrade",
            "rbac-change",
            "session-connect",
            "route-admission",
            "resource-share",
            "broker-effect",
            "process-effect",
            "state-reset",
            "privileged",
            "unprivileged",
        ],
    ),
    (
        "provider_class",
        &[
            "system-core",
            "system-minijail",
            "system-systemd",
            "observability-otel",
        ],
    ),
    (
        "phase",
        &[
            "pending", "ready", "degraded", "failed", "unknown", "revoked",
        ],
    ),
    ("signal", &["metric", "trace", "log"]),
    (
        "event",
        &[
            "accepted",
            "rejected",
            "connect",
            "reconnect",
            "close",
            "launch",
            "stop",
            "adopt",
            "quarantine",
            "start",
            "ready",
            "degraded",
            "failed",
            "finalized",
            "flush",
            "shutdown",
            "attach",
            "detach",
            "export",
            "buffer",
        ],
    ),
    (
        "reason",
        &[
            "buffer_full",
            "export_error",
            "policy_violation",
            "ingress_quarantine",
            "auth",
            "quota",
            "conflict",
            "invalid",
            "schema",
        ],
    ),
    (
        "ingress",
        &["emitter_unix", "otlp_unix", "otlp_vsock", "import_stream"],
    ),
    (
        "error_class",
        &[
            "none",
            "key_not_allowlisted",
            "key_forbidden",
            "key_suffix_forbidden",
            "value_identity",
            "malformed",
            "oversize",
            "transport",
            "auth",
            "protocol",
            "timeout",
            "capability",
            "capacity",
            "stale-session",
            "already-attached",
            "not-found",
            "output-gap",
            "offset-mismatch",
            "terminal-closed",
            "invalid-size",
            "helper-unavailable",
            "helper-stale",
            "user-manager",
            "environment",
            "executable",
            "scope-create",
            "scope-identity",
            "graphical-session",
            "wayland",
            "proxy",
            "operation-conflict",
            "guest",
            "internal",
        ],
    ),
    ("stop_class", &["graceful", "forced"]),
    ("class", &["exited", "signaled", "killed"]),
    ("arbitration", &["exclusive", "shared", "multiplexed"]),
    (
        "state",
        &[
            "advertised",
            "ready",
            "revoking",
            "degraded",
            "pending",
            "reachable",
            "bound",
            "revoked",
            "active",
            "not-applicable",
            "helper-unavailable",
            "helper-stale",
            "user-manager-unavailable",
            "graphical-session-inactive",
            "wayland-unavailable",
            "proxy-unavailable",
        ],
    ),
    (
        "component",
        &[
            "shell", "exec", "workload", "launcher", "helper", "scope", "proxy",
        ],
    ),
    ("component_type", &["controller", "service", "worker"]),
    (
        "disposition",
        &["none", "reload", "restart", "recycle", "replace"],
    ),
];

/// One canonical metric family descriptor.
///
/// The labels identify the complete data-point shape and the exact closed
/// value domain for each key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDescriptorSpec {
    /// Stable metric family name.
    pub name: &'static str,
    /// Complete, ordered label-key and value-domain set.
    pub labels: &'static [(&'static str, &'static [&'static str])],
}

/// Canonical metric family registry shared by telemetry producers and the
/// observability Provider ingress.
pub const METRIC_DESCRIPTOR_REGISTRY: &[MetricDescriptorSpec] = &[
    descriptor(
        "d2b_telemetry_drop_total",
        &[
            ("signal", &["metric", "trace", "log"]),
            (
                "reason",
                &[
                    "buffer_full",
                    "export_error",
                    "policy_violation",
                    "ingress_quarantine",
                ],
            ),
        ],
    ),
    descriptor(
        "d2b_telemetry_export_total",
        &[
            ("signal", &["metric", "trace", "log"]),
            ("outcome", &["ok", "error"]),
        ],
    ),
    descriptor(
        "d2b_otel_ingress_policy_total",
        &[
            (
                "ingress",
                &["emitter_unix", "otlp_unix", "otlp_vsock", "import_stream"],
            ),
            ("outcome", &["accepted", "rejected", "quarantined"]),
            (
                "error_class",
                &[
                    "none",
                    "key_not_allowlisted",
                    "key_forbidden",
                    "key_suffix_forbidden",
                    "value_identity",
                    "malformed",
                    "oversize",
                ],
            ),
        ],
    ),
    descriptor(
        "d2b_api_request_total",
        &[
            ("verb", API_VERBS),
            ("resource_type", RESOURCE_TYPES),
            (
                "outcome",
                &[
                    "ok",
                    "conflict",
                    "invalid",
                    "denied",
                    "not_found",
                    "quota",
                    "error",
                ],
            ),
        ],
    ),
    descriptor(
        "d2b_api_request_duration_seconds",
        &[("verb", API_VERBS), ("resource_type", RESOURCE_TYPES)],
    ),
    descriptor("d2b_api_watch_active", &[]),
    descriptor(
        "d2b_api_admission_rejected_total",
        &[(
            "reason",
            &["auth", "quota", "conflict", "invalid", "schema"],
        )],
    ),
    descriptor(
        "d2b_bus_route_total",
        &[
            ("service", SERVICE_LABELS),
            ("direction", DIRECTIONS),
            ("outcome", &["ok", "denied", "not_found", "error"]),
        ],
    ),
    descriptor(
        "d2b_bus_route_duration_seconds",
        &[("service", SERVICE_LABELS), ("direction", DIRECTIONS)],
    ),
    descriptor("d2b_bus_session_active", &[("transport", TRANSPORTS)]),
    descriptor(
        "d2b_bus_registration_total",
        &[
            ("direction", DIRECTIONS),
            ("outcome", &["accepted", "rejected"]),
        ],
    ),
    descriptor("d2b_bus_stream_active", &[("direction", DIRECTIONS)]),
    descriptor(
        "d2b_bus_stream_total",
        &[
            ("direction", DIRECTIONS),
            ("outcome", &["accepted", "rejected", "abandoned"]),
        ],
    ),
    descriptor("d2b_bus_credit_bytes", &[("direction", DIRECTIONS)]),
    descriptor(
        "d2b_bus_backpressure_total",
        &[
            ("direction", DIRECTIONS),
            ("kind", &["control", "stream"]),
            ("reason", &["buffer_full", "quota"]),
        ],
    ),
    descriptor(
        "d2b_bus_rejection_total",
        &[
            ("direction", DIRECTIONS),
            ("outcome", &["denied", "not_found", "error", "quota"]),
        ],
    ),
    descriptor(
        "d2b_bus_disconnect_total",
        &[
            ("direction", DIRECTIONS),
            ("outcome", &["abandoned", "cancel", "revoked", "error"]),
        ],
    ),
    descriptor(
        "d2b_controller_reconcile_total",
        &[
            ("handler", HANDLERS),
            ("outcome", &["ok", "requeue", "conflict", "error"]),
        ],
    ),
    descriptor(
        "d2b_controller_reconcile_duration_seconds",
        &[
            ("handler", HANDLERS),
            ("outcome", &["ok", "requeue", "conflict", "error"]),
        ],
    ),
    descriptor("d2b_controller_queue_depth", &[("handler", HANDLERS)]),
    descriptor(
        "d2b_controller_hint_to_handler_seconds",
        &[("handler", HANDLERS)],
    ),
    descriptor(
        "d2b_controller_watch_revision_lag",
        &[("handler", HANDLERS)],
    ),
    descriptor(
        "d2b_provider_component_phase",
        &[
            ("component_type", &["controller", "service", "worker"]),
            (
                "phase",
                &["pending", "ready", "degraded", "failed", "unknown"],
            ),
        ],
    ),
    descriptor(
        "d2b_store_write_duration_seconds",
        &[
            ("kind", &["single", "group"]),
            ("outcome", &["ok", "conflict", "error"]),
        ],
    ),
    descriptor(
        "d2b_store_read_duration_seconds",
        &[("operation", &["get", "list", "scan"])],
    ),
    descriptor("d2b_store_group_commit_size", &[]),
    descriptor(
        "d2b_store_conflict_total",
        &[("resource_type", RESOURCE_TYPES)],
    ),
    descriptor("d2b_store_watch_active", &[]),
    descriptor("d2b_store_revision", &[]),
    descriptor(
        "d2b_store_compaction_duration_seconds",
        &[("outcome", &["ok", "error"])],
    ),
    descriptor(
        "d2b_store_backup_duration_seconds",
        &[("outcome", &["ok", "error"])],
    ),
    descriptor(
        "d2b_store_queue_depth",
        &[("operation", &["write", "read"])],
    ),
    descriptor(
        "d2b_process_launch_total",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("domain", &["system", "user"]),
            ("outcome", &["ok", "error", "quota"]),
        ],
    ),
    descriptor(
        "d2b_process_launch_duration_seconds",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("domain", &["system", "user"]),
        ],
    ),
    descriptor(
        "d2b_process_active",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("domain", &["system", "user"]),
        ],
    ),
    descriptor(
        "d2b_process_restart_total",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("class", &["exited", "signaled", "killed"]),
        ],
    ),
    descriptor(
        "d2b_process_adoption_total",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("outcome", &["ok", "degraded", "error"]),
        ],
    ),
    descriptor("d2b_process_pidfd_active", &[]),
    descriptor(
        "d2b_process_stop_duration_seconds",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("stop_class", &["graceful", "forced"]),
            ("outcome", &["ok", "error"]),
        ],
    ),
    descriptor(
        "d2b_process_ready_duration_seconds",
        &[
            ("provider", PROCESS_PROVIDERS),
            ("domain", &["system", "user"]),
        ],
    ),
    descriptor(
        "d2b_session_connect_total",
        &[
            ("profile", &["NN", "KK", "IKpsk2"]),
            ("purpose_class", &["local", "enrolled", "bootstrap"]),
            (
                "outcome",
                &["ok", "auth", "transcript", "policy", "timeout", "error"],
            ),
        ],
    ),
    descriptor(
        "d2b_session_reconnect_total",
        &[("outcome", &["ok", "error", "abandoned"])],
    ),
    descriptor(
        "d2b_session_record_total",
        &[
            ("direction", &["send", "recv"]),
            ("kind", &["control", "ttrpc", "stream", "attachment"]),
        ],
    ),
    descriptor("d2b_session_active", &[("transport", TRANSPORTS)]),
];

const API_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
    "use-credential",
    "admin-credential",
];

const RESOURCE_TYPES: &[&str] = &[
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "Host",
    "Guest",
    "Process",
    "EphemeralProcess",
    "Volume",
    "Network",
    "Device",
    "User",
    "Credential",
    "Endpoint",
    "ResourceExport",
    "ResourceImport",
    "vendor",
];

const SERVICE_LABELS: &[&str] = &[
    "bus",
    "d2b.resource.v3",
    "d2b.controller.v3",
    "d2b.provider.v3",
    "d2b.audit.v3",
    "d2b.support.v3",
    "d2b.credential.v3",
    "d2b.zone.v3",
    "d2b.zonelink.v3",
    "d2b.volume.v3",
];

const DIRECTIONS: &[&str] = &["local", "host", "guest", "zone_link"];
const TRANSPORTS: &[&str] = &["unix", "vsock", "zone_link"];
const PROCESS_PROVIDERS: &[&str] = &["minijail", "systemd"];
const HANDLERS: &[&str] = &[
    "configuration",
    "api_catalog",
    "authz",
    "provider",
    "controller_registration",
    "ownership",
    "watch_maintenance",
    "ephemeral_cleanup",
    "zone_link",
    "budget",
    "store_lifecycle",
    "system_core_host",
    "system_core_user",
];

const fn descriptor(
    name: &'static str,
    labels: &'static [(&'static str, &'static [&'static str])],
) -> MetricDescriptorSpec {
    MetricDescriptorSpec { name, labels }
}

/// Return the closed value domain for one accepted label key.
pub fn allowed_values(key: &str) -> Option<&'static [&'static str]> {
    METRIC_LABEL_POLICY
        .iter()
        .find_map(|(candidate, values)| (*candidate == key).then_some(*values))
}

/// Resolve one metric family against the canonical registry.
pub fn metric_descriptor(name: &str) -> Option<&'static MetricDescriptorSpec> {
    METRIC_DESCRIPTOR_REGISTRY
        .iter()
        .find(|descriptor| descriptor.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn descriptor_registry_is_closed_and_uses_known_label_domains() {
        let mut names = BTreeSet::new();
        for descriptor in METRIC_DESCRIPTOR_REGISTRY {
            assert!(names.insert(descriptor.name));
            for (key, values) in descriptor.labels {
                let Some(allowed) = allowed_values(key) else {
                    panic!("{key}");
                };
                assert!(values.iter().all(|value| allowed.contains(value)), "{key}");
            }
        }
        assert!(metric_descriptor("d2b_otel_ingress_policy_total").is_some());
        assert!(metric_descriptor("d2b_unregistered_total").is_none());
    }
}
