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
        ],
    ),
    ("component_type", &["controller", "service", "worker"]),
    (
        "disposition",
        &["none", "reload", "restart", "recycle", "replace"],
    ),
];

/// Return the closed value domain for one accepted label key.
pub fn allowed_values(key: &str) -> Option<&'static [&'static str]> {
    METRIC_LABEL_POLICY
        .iter()
        .find_map(|(candidate, values)| (*candidate == key).then_some(*values))
}
