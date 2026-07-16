use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
struct Component {
    id: &'static str,
    owned_files: &'static [&'static str],
    reserved_prefixes: &'static [&'static str],
    dependencies: &'static [&'static str],
    frozen_inputs: &'static [&'static str],
    service_package: Option<&'static str>,
    endpoint_purpose: Option<&'static str>,
    endpoint_role: Option<&'static str>,
    frozen_parent_blockers: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct FrozenParentBlocker {
    id: &'static str,
    authority: &'static str,
    paths: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct FrozenInput {
    id: &'static str,
    paths: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct LegacyBoundary {
    id: &'static str,
    owner: &'static str,
    call_graph: &'static [&'static str],
    legacy_handshake: &'static str,
    disposition: &'static str,
    frozen_parent_blocker: Option<&'static str>,
}

const OWNED_PACKAGE_PREFIXES: &[&str] = &[
    "packages/d2b-activation-helper/",
    "packages/d2b-clipd/",
    "packages/d2b-guest-shell-runner/",
    "packages/d2b-host-activation-helper/",
    "packages/d2b-notify/",
    "packages/d2b-one-shot-helper/",
    "packages/d2b-runtime-systemd-user/",
    "packages/d2b-security-key-helper/",
    "packages/d2b-shell-supervisor/",
    "packages/d2b-sk-frontend/",
    "packages/d2b-systemd-user-agent/",
    "packages/d2b-tty-helper/",
    "packages/d2b-unsafe-local-helper/",
    "packages/d2b-userd/",
    "packages/d2b-wayland-proxy/",
    "packages/d2b-wlcontrol/",
];

const BLOCKERS: &[FrozenParentBlocker] = &[
    FrozenParentBlocker {
        id: "activation-bootstrap",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-priv-broker/src/live_handlers.rs",
        ],
    },
    FrozenParentBlocker {
        id: "clipboard-endpoint",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-client/src/host_socket.rs",
            "packages/d2b-client/src/service.rs",
            "packages/d2b/src/lib.rs",
        ],
    },
    FrozenParentBlocker {
        id: "guest-shell-bootstrap",
        authority: "core-control-parent",
        paths: &["packages/d2b-guestd/src/service.rs"],
    },
    FrozenParentBlocker {
        id: "notify-endpoint",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-client/src/host_socket.rs",
            "packages/d2b-client/src/service.rs",
            "packages/d2b/src/lib.rs",
            "packages/d2bd/src/lib.rs",
        ],
    },
    FrozenParentBlocker {
        id: "runtime-endpoint",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-client/src/host_socket.rs",
            "packages/d2b-client/src/service.rs",
            "packages/d2bd/src/lib.rs",
            "packages/d2bd/src/unsafe_local_helper.rs",
        ],
    },
    FrozenParentBlocker {
        id: "security-key-bootstrap",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-priv-broker/src/ops/security_key.rs",
            "packages/d2bd/src/lib.rs",
            "packages/d2bd/src/security_key.rs",
        ],
    },
    FrozenParentBlocker {
        id: "user-agent-endpoint",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-client/src/host_socket.rs",
            "packages/d2b-client/src/service.rs",
            "packages/d2b/src/lib.rs",
            "packages/d2bd/src/lib.rs",
        ],
    },
    FrozenParentBlocker {
        id: "wayland-bootstrap",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-host/src/wayland_proxy_argv.rs",
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "packages/d2bd/src/lib.rs",
        ],
    },
    FrozenParentBlocker {
        id: "workspace-membership",
        authority: "shared-root",
        paths: &["packages/Cargo.lock", "packages/Cargo.toml"],
    },
];

const FROZEN_INPUTS: &[FrozenInput] = &[
    FrozenInput {
        id: "component-session",
        paths: &[
            "packages/d2b-contracts/src/v2_component_session.rs",
            "packages/d2b-session-unix/src/lib.rs",
            "packages/d2b-session/src/lib.rs",
        ],
    },
    FrozenInput {
        id: "credential-placement",
        paths: &[
            "packages/d2b-contracts/src/v2_provider.rs",
            "packages/d2b-provider-credential-secret-service/src/lib.rs",
        ],
    },
    FrozenInput {
        id: "device-provider",
        paths: &["packages/d2b-provider-device-host-mediated/src/lib.rs"],
    },
    FrozenInput {
        id: "display-provider",
        paths: &["packages/d2b-provider-display-wayland/src/lib.rs"],
    },
    FrozenInput {
        id: "observability-result",
        paths: &["packages/d2b-provider-observability-local/src/lib.rs"],
    },
    FrozenInput {
        id: "provider-framework",
        paths: &[
            "packages/d2b-provider-toolkit/src/lib.rs",
            "packages/d2b-provider/src/lib.rs",
        ],
    },
    FrozenInput {
        id: "service-contracts",
        paths: &[
            "docs/reference/v2-services.json",
            "packages/d2b-contracts/src/v2_services.rs",
        ],
    },
    FrozenInput {
        id: "transport-contract",
        paths: &["packages/d2b-provider-transport-local/src/lib.rs"],
    },
];

const COMPONENTS: &[Component] = &[
    Component {
        id: "activation-one-shot",
        owned_files: &[
            "packages/d2b-host-activation-helper/Cargo.toml",
            "packages/d2b-host-activation-helper/src/lib.rs",
            "packages/d2b-host-activation-helper/src/main.rs",
            "packages/d2b-host-activation-helper/src/services/mod.rs",
        ],
        reserved_prefixes: &[
            "packages/d2b-activation-helper/",
            "packages/d2b-host-activation-helper/src/services/activation/",
            "packages/d2b-one-shot-helper/",
        ],
        dependencies: &[],
        frozen_inputs: &["component-session", "service-contracts"],
        service_package: Some("d2b.activation.v2"),
        endpoint_purpose: Some("activation-helper"),
        endpoint_role: Some("activation-helper"),
        frozen_parent_blockers: &["activation-bootstrap", "workspace-membership"],
    },
    Component {
        id: "clipboard-bridge",
        owned_files: &[
            "packages/d2b-clipd/src/fd.rs",
            "packages/d2b-wayland-proxy/src/bridge.rs",
            "packages/d2b-wayland-proxy/src/clipboard.rs",
        ],
        reserved_prefixes: &["packages/d2b-clipd/src/services/bridge/"],
        dependencies: &["clipboard-control"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.clipboard.v2"),
        endpoint_purpose: Some("clipboard-bridge"),
        endpoint_role: Some("clipboard-daemon"),
        frozen_parent_blockers: &[],
    },
    Component {
        id: "clipboard-composition",
        owned_files: &[
            "packages/d2b-clipd/Cargo.toml",
            "packages/d2b-clipd/src/lib.rs",
            "packages/d2b-clipd/src/main.rs",
            "packages/d2b-clipd/src/services/mod.rs",
            "packages/d2b-clipd/tests/daemon_cli.rs",
        ],
        reserved_prefixes: &[],
        dependencies: &["clipboard-bridge", "clipboard-control", "clipboard-picker"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["clipboard-endpoint"],
    },
    Component {
        id: "clipboard-control",
        owned_files: &[
            "packages/d2b-clipd/src/audit.rs",
            "packages/d2b-clipd/src/bin/d2b-clip-debug.rs",
            "packages/d2b-clipd/src/fallback.rs",
            "packages/d2b-clipd/src/host.rs",
            "packages/d2b-clipd/src/niri.rs",
            "packages/d2b-clipd/src/notifications.rs",
            "packages/d2b-clipd/src/policy.rs",
            "packages/d2b-clipd/src/virtual_keyboard.rs",
            "packages/d2b-clipd/src/wayland.rs",
            "packages/d2b-clipd/tests/test_pipe.rs",
        ],
        reserved_prefixes: &["packages/d2b-clipd/src/services/control/"],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.clipboard.v2"),
        endpoint_purpose: Some("clipboard-control"),
        endpoint_role: Some("clipboard-daemon"),
        frozen_parent_blockers: &["clipboard-endpoint"],
    },
    Component {
        id: "clipboard-picker",
        owned_files: &[
            "packages/d2b-clipd/src/framing.rs",
            "packages/d2b-clipd/src/picker.rs",
            "packages/d2b-clipd/src/protocol.rs",
        ],
        reserved_prefixes: &["packages/d2b-clipd/src/services/picker/"],
        dependencies: &["clipboard-control"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.clipboard.picker.v2"),
        endpoint_purpose: Some("clipboard-picker"),
        endpoint_role: Some("clipboard-picker"),
        frozen_parent_blockers: &[],
    },
    Component {
        id: "desktop-actions",
        owned_files: &[
            "packages/d2b-notify/src/nonce.rs",
            "packages/d2b-notify/src/wlcontrol.rs",
        ],
        reserved_prefixes: &[
            "packages/d2b-notify/src/services/actions/",
            "packages/d2b-wlcontrol/",
        ],
        dependencies: &["desktop-observer"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.notify.v2"),
        endpoint_purpose: Some("desktop-observer"),
        endpoint_role: Some("desktop-observer"),
        frozen_parent_blockers: &["notify-endpoint", "workspace-membership"],
    },
    Component {
        id: "desktop-composition",
        owned_files: &[
            "packages/d2b-notify/Cargo.toml",
            "packages/d2b-notify/src/lib.rs",
            "packages/d2b-notify/src/services/mod.rs",
        ],
        reserved_prefixes: &[],
        dependencies: &["desktop-actions", "desktop-observer"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["notify-endpoint"],
    },
    Component {
        id: "desktop-observer",
        owned_files: &[
            "packages/d2b-notify/src/bin/waybar_helper.rs",
            "packages/d2b-notify/src/events.rs",
            "packages/d2b-notify/src/notifications.rs",
            "packages/d2b-notify/src/state.rs",
            "packages/d2b-notify/src/waybar.rs",
        ],
        reserved_prefixes: &["packages/d2b-notify/src/services/observer/"],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.notify.v2"),
        endpoint_purpose: Some("desktop-observer"),
        endpoint_role: Some("desktop-observer"),
        frozen_parent_blockers: &["notify-endpoint"],
    },
    Component {
        id: "retained-guest-shell",
        owned_files: &[
            "packages/d2b-guest-shell-runner/Cargo.toml",
            "packages/d2b-guest-shell-runner/deny.toml",
            "packages/d2b-guest-shell-runner/src/cli.rs",
            "packages/d2b-guest-shell-runner/src/lib.rs",
            "packages/d2b-guest-shell-runner/src/libshpool_bridge.rs",
            "packages/d2b-guest-shell-runner/src/main.rs",
            "packages/d2b-guest-shell-runner/src/name.rs",
            "packages/d2b-guest-shell-runner/src/output.rs",
            "packages/d2b-guest-shell-runner/src/services/mod.rs",
            "packages/d2b-guest-shell-runner/src/socket.rs",
            "packages/d2b-guest-shell-runner/tests/cli.rs",
        ],
        reserved_prefixes: &["packages/d2b-guest-shell-runner/src/services/retained_shell/"],
        dependencies: &[],
        frozen_inputs: &["component-session", "service-contracts"],
        service_package: Some("d2b.guest.v2"),
        endpoint_purpose: Some("guest-control"),
        endpoint_role: Some("guest-agent"),
        frozen_parent_blockers: &["guest-shell-bootstrap"],
    },
    Component {
        id: "runtime-composition",
        owned_files: &[
            "packages/d2b-unsafe-local-helper/Cargo.toml",
            "packages/d2b-unsafe-local-helper/src/lib.rs",
            "packages/d2b-unsafe-local-helper/src/main.rs",
            "packages/d2b-unsafe-local-helper/src/services/mod.rs",
        ],
        reserved_prefixes: &[],
        dependencies: &[
            "runtime-systemd-user",
            "shell-supervisor",
            "tty-one-shot",
            "wayland-composition",
        ],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["runtime-endpoint"],
    },
    Component {
        id: "runtime-systemd-user",
        owned_files: &[
            "packages/d2b-unsafe-local-helper/src/environment.rs",
            "packages/d2b-unsafe-local-helper/src/protocol.rs",
            "packages/d2b-unsafe-local-helper/src/runtime.rs",
            "packages/d2b-unsafe-local-helper/src/systemd.rs",
        ],
        reserved_prefixes: &[
            "packages/d2b-runtime-systemd-user/",
            "packages/d2b-systemd-user-agent/",
            "packages/d2b-unsafe-local-helper/src/services/runtime_systemd_user/",
        ],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.runtime.systemd-user.v2"),
        endpoint_purpose: Some("runtime-systemd-user"),
        endpoint_role: Some("runtime-systemd-user-agent"),
        frozen_parent_blockers: &["runtime-endpoint", "workspace-membership"],
    },
    Component {
        id: "security-key-controller",
        owned_files: &[],
        reserved_prefixes: &["packages/d2b-security-key-helper/"],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "device-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.security-key.v2"),
        endpoint_purpose: Some("security-key"),
        endpoint_role: Some("security-key-controller"),
        frozen_parent_blockers: &["security-key-bootstrap", "workspace-membership"],
    },
    Component {
        id: "security-key-frontend",
        owned_files: &[
            "packages/d2b-sk-frontend/Cargo.toml",
            "packages/d2b-sk-frontend/src/framing.rs",
            "packages/d2b-sk-frontend/src/lib.rs",
            "packages/d2b-sk-frontend/src/main.rs",
            "packages/d2b-sk-frontend/src/services/mod.rs",
            "packages/d2b-sk-frontend/src/uhid.rs",
            "packages/d2b-sk-frontend/src/vsock.rs",
        ],
        reserved_prefixes: &["packages/d2b-sk-frontend/src/services/security_key/"],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "device-provider",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.security-key.v2"),
        endpoint_purpose: Some("security-key"),
        endpoint_role: Some("security-key-frontend"),
        frozen_parent_blockers: &["security-key-bootstrap"],
    },
    Component {
        id: "shell-supervisor",
        owned_files: &[
            "packages/d2b-unsafe-local-helper/src/output_ring.rs",
            "packages/d2b-unsafe-local-helper/src/shell_runtime.rs",
            "packages/d2b-unsafe-local-helper/src/shell_socket.rs",
            "packages/d2b-unsafe-local-helper/src/shell_supervisor.rs",
            "packages/d2b-unsafe-local-helper/src/supervisor_protocol.rs",
            "packages/d2b-unsafe-local-helper/tests/shell_supervisor.rs",
        ],
        reserved_prefixes: &[
            "packages/d2b-shell-supervisor/",
            "packages/d2b-unsafe-local-helper/src/services/shell/",
        ],
        dependencies: &["runtime-systemd-user"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.shell.v2"),
        endpoint_purpose: Some("shell-supervisor"),
        endpoint_role: Some("shell-supervisor"),
        frozen_parent_blockers: &["runtime-endpoint", "workspace-membership"],
    },
    Component {
        id: "tty-one-shot",
        owned_files: &["packages/d2b-unsafe-local-helper/src/tty_exec.rs"],
        reserved_prefixes: &[
            "packages/d2b-tty-helper/",
            "packages/d2b-unsafe-local-helper/src/services/tty/",
        ],
        dependencies: &["runtime-systemd-user"],
        frozen_inputs: &["component-session", "service-contracts"],
        service_package: Some("d2b.tty.v2"),
        endpoint_purpose: Some("tty-helper"),
        endpoint_role: Some("tty-helper"),
        frozen_parent_blockers: &["runtime-endpoint", "workspace-membership"],
    },
    Component {
        id: "user-secrets",
        owned_files: &[
            "packages/d2b-userd/Cargo.toml",
            "packages/d2b-userd/src/lib.rs",
            "packages/d2b-userd/src/main.rs",
            "packages/d2b-userd/src/services/mod.rs",
            "packages/d2b-userd/tests/edge_composition_policy.rs",
            "packages/d2b-userd/tests/fail_closed.rs",
        ],
        reserved_prefixes: &["packages/d2b-userd/src/services/user/"],
        dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "credential-placement",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.user.v2"),
        endpoint_purpose: Some("user-agent"),
        endpoint_role: Some("user-agent"),
        frozen_parent_blockers: &["user-agent-endpoint"],
    },
    Component {
        id: "wayland-composition",
        owned_files: &[
            "packages/d2b-wayland-proxy/Cargo.toml",
            "packages/d2b-wayland-proxy/src/filter.rs",
            "packages/d2b-wayland-proxy/src/lib.rs",
            "packages/d2b-wayland-proxy/src/main.rs",
            "packages/d2b-wayland-proxy/src/services/mod.rs",
        ],
        reserved_prefixes: &[],
        dependencies: &["clipboard-bridge", "wayland-control"],
        frozen_inputs: &[
            "component-session",
            "display-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["wayland-bootstrap"],
    },
    Component {
        id: "wayland-control",
        owned_files: &[
            "packages/d2b-wayland-proxy/src/attribution.rs",
            "packages/d2b-wayland-proxy/src/decoration.rs",
            "packages/d2b-wayland-proxy/src/diag.rs",
            "packages/d2b-wayland-proxy/src/dmabuf.rs",
            "packages/d2b-wayland-proxy/src/identity.rs",
            "packages/d2b-wayland-proxy/src/policy.rs",
            "packages/d2b-wayland-proxy/src/readiness.rs",
            "packages/d2b-wayland-proxy/src/terminal.rs",
        ],
        reserved_prefixes: &["packages/d2b-wayland-proxy/src/services/wayland/"],
        dependencies: &["clipboard-bridge"],
        frozen_inputs: &[
            "component-session",
            "display-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_package: Some("d2b.wayland.v2"),
        endpoint_purpose: Some("wayland-proxy"),
        endpoint_role: Some("wayland-proxy"),
        frozen_parent_blockers: &["wayland-bootstrap"],
    },
];

const LEGACY_BOUNDARIES: &[LegacyBoundary] = &[
    LegacyBoundary {
        id: "activation-argv-and-stdin",
        owner: "activation-one-shot",
        call_graph: &[
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-host-activation-helper/src/main.rs",
        ],
        legacy_handshake: "argv verbs plus untyped stdin JSON or process exit status",
        disposition: "fold-or-component-session",
        frozen_parent_blocker: Some("activation-bootstrap"),
    },
    LegacyBoundary {
        id: "clipboard-cli-control",
        owner: "clipboard-control",
        call_graph: &["packages/d2b/src/lib.rs", "packages/d2b-clipd/src/main.rs"],
        legacy_handshake: "self-bound Unix stream with newline JSON arm request and untyped response",
        disposition: "d2b.clipboard.v2",
        frozen_parent_blocker: Some("clipboard-endpoint"),
    },
    LegacyBoundary {
        id: "clipboard-picker-child",
        owner: "clipboard-picker",
        call_graph: &[
            "packages/d2b-clipd/src/framing.rs",
            "packages/d2b-clipd/src/picker.rs",
            "packages/d2b-clipd/src/protocol.rs",
            "packages/d2b-clipd/src/main.rs",
        ],
        legacy_handshake: "inherited Unix stream with protocol-1 client hello and newline JSON",
        disposition: "d2b.clipboard.picker.v2",
        frozen_parent_blocker: None,
    },
    LegacyBoundary {
        id: "clipboard-proxy-bridge",
        owner: "clipboard-bridge",
        call_graph: &[
            "packages/d2b-wayland-proxy/src/bridge.rs",
            "packages/d2b-wayland-proxy/src/filter.rs",
            "packages/d2b-clipd/src/main.rs",
        ],
        legacy_handshake: "derived-path Unix stream with unversioned newline JSON and SCM_RIGHTS",
        disposition: "d2b.clipboard.v2",
        frozen_parent_blocker: None,
    },
    LegacyBoundary {
        id: "guest-shell-parent-control",
        owner: "retained-guest-shell",
        call_graph: &[
            "packages/d2b-guestd/src/service.rs",
            "packages/d2b-guest-shell-runner/src/main.rs",
        ],
        legacy_handshake: "parent argv and process status around an external libshpool data plane",
        disposition: "guest-service-or-delete",
        frozen_parent_blocker: Some("guest-shell-bootstrap"),
    },
    LegacyBoundary {
        id: "notify-projection-actions",
        owner: "desktop-actions",
        call_graph: &[
            "packages/d2b-notify/src/state.rs",
            "packages/d2b-notify/src/wlcontrol.rs",
            "packages/d2b/src/lib.rs",
        ],
        legacy_handshake: "durable state projection plus callback nonce forwarded through the CLI",
        disposition: "d2b.notify.v2",
        frozen_parent_blocker: Some("notify-endpoint"),
    },
    LegacyBoundary {
        id: "runtime-daemon-helper",
        owner: "runtime-systemd-user",
        call_graph: &[
            "packages/d2bd/src/unsafe_local_helper.rs",
            "packages/d2b-unsafe-local-helper/src/protocol.rs",
        ],
        legacy_handshake: "helper protocol 3 hello, generation, snapshot, heartbeat, JSON frames, and SCM_RIGHTS",
        disposition: "d2b.runtime.systemd-user.v2",
        frozen_parent_blocker: Some("runtime-endpoint"),
    },
    LegacyBoundary {
        id: "runtime-scope-bootstrap",
        owner: "runtime-systemd-user",
        call_graph: &["packages/d2b-unsafe-local-helper/src/runtime.rs"],
        legacy_handshake: "length-prefixed stdin JSON, release byte, and stdout readiness byte",
        disposition: "inherited-component-session",
        frozen_parent_blocker: None,
    },
    LegacyBoundary {
        id: "security-key-report-stream",
        owner: "security-key-frontend",
        call_graph: &[
            "packages/d2b-sk-frontend/src/framing.rs",
            "packages/d2b-sk-frontend/src/vsock.rs",
            "packages/d2bd/src/security_key.rs",
            "packages/d2bd/src/lib.rs",
        ],
        legacy_handshake: "raw AF_VSOCK/Unix relay with little-endian 64-byte report framing and no session handshake",
        disposition: "d2b.security-key.v2-named-stream",
        frozen_parent_blocker: Some("security-key-bootstrap"),
    },
    LegacyBoundary {
        id: "shell-supervisor-bootstrap",
        owner: "shell-supervisor",
        call_graph: &["packages/d2b-unsafe-local-helper/src/shell_supervisor.rs"],
        legacy_handshake: "length-prefixed stdin JSON, release byte, and stdout readiness byte",
        disposition: "inherited-component-session",
        frozen_parent_blocker: None,
    },
    LegacyBoundary {
        id: "shell-supervisor-control",
        owner: "shell-supervisor",
        call_graph: &[
            "packages/d2b-unsafe-local-helper/src/shell_runtime.rs",
            "packages/d2b-unsafe-local-helper/src/shell_socket.rs",
            "packages/d2b-unsafe-local-helper/src/shell_supervisor.rs",
            "packages/d2b-unsafe-local-helper/src/supervisor_protocol.rs",
        ],
        legacy_handshake: "self-bound per-shell Unix stream with protocol-1 length-prefixed JSON and terminal framing",
        disposition: "d2b.shell.v2-named-stream",
        frozen_parent_blocker: None,
    },
    LegacyBoundary {
        id: "tty-status-channel",
        owner: "tty-one-shot",
        call_graph: &["packages/d2b-unsafe-local-helper/src/tty_exec.rs"],
        legacy_handshake: "inherited terminal stdin and one-byte stdout setup status",
        disposition: "d2b.tty.v2",
        frozen_parent_blocker: Some("runtime-endpoint"),
    },
    LegacyBoundary {
        id: "wayland-display-listener",
        owner: "wayland-control",
        call_graph: &[
            "packages/d2b-host/src/wayland_proxy_argv.rs",
            "packages/d2b-wayland-proxy/src/main.rs",
        ],
        legacy_handshake: "proxy self-binds display path while external Wayland remains the data plane",
        disposition: "d2b.wayland.v2-control",
        frozen_parent_blocker: Some("wayland-bootstrap"),
    },
    LegacyBoundary {
        id: "wayland-runtime-readiness",
        owner: "wayland-control",
        call_graph: &[
            "packages/d2b-wayland-proxy/src/readiness.rs",
            "packages/d2b-unsafe-local-helper/src/runtime.rs",
        ],
        legacy_handshake: "derived-path Unix stream with protocol-1 newline JSON readiness events",
        disposition: "d2b.wayland.v2",
        frozen_parent_blocker: None,
    },
];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

fn collect_files(root: &Path, relative: &Path, out: &mut BTreeSet<String>) {
    let absolute = root.join(relative);
    if !absolute.exists() {
        return;
    }
    for entry in fs::read_dir(&absolute).expect("read owned package tree") {
        let entry = entry.expect("owned package entry");
        let file_name = entry.file_name();
        if file_name == "target" || file_name == ".git" {
            continue;
        }
        let child = relative.join(file_name);
        if entry.file_type().expect("owned package file type").is_dir() {
            collect_files(root, &child, out);
        } else {
            out.insert(child.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn owner_for(path: &str) -> Vec<&'static str> {
    COMPONENTS
        .iter()
        .filter(|component| {
            component.owned_files.contains(&path)
                || component
                    .reserved_prefixes
                    .iter()
                    .any(|prefix| path.starts_with(prefix))
        })
        .map(|component| component.id)
        .collect()
}

#[test]
fn edge_files_have_exactly_one_local_owner() {
    let root = repository_root();
    let mut files = BTreeSet::new();
    for prefix in OWNED_PACKAGE_PREFIXES {
        collect_files(&root, Path::new(prefix), &mut files);
    }
    assert!(!files.is_empty());

    for file in files {
        assert_eq!(
            owner_for(&file).len(),
            1,
            "{file} must have exactly one edge component owner"
        );
    }

    for component in COMPONENTS {
        for file in component.owned_files {
            assert!(root.join(file).is_file(), "{} is missing", file);
            assert_eq!(owner_for(file), vec![component.id]);
        }
    }
}

#[test]
fn component_dependencies_are_known_and_acyclic() {
    let components = COMPONENTS
        .iter()
        .map(|component| (component.id, component))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(components.len(), COMPONENTS.len(), "duplicate component id");

    fn visit(
        id: &'static str,
        components: &BTreeMap<&'static str, &Component>,
        active: &mut BTreeSet<&'static str>,
        complete: &mut BTreeSet<&'static str>,
    ) {
        if complete.contains(id) {
            return;
        }
        assert!(active.insert(id), "component dependency cycle at {id}");
        let component = components.get(id).expect("known component");
        for dependency in component.dependencies {
            assert!(
                components.contains_key(dependency),
                "{id} has unknown dependency {dependency}"
            );
            visit(dependency, components, active, complete);
        }
        active.remove(id);
        complete.insert(id);
    }

    let mut complete = BTreeSet::new();
    for id in components.keys().copied() {
        visit(id, &components, &mut BTreeSet::new(), &mut complete);
    }
}

#[test]
fn service_composition_uses_only_frozen_contract_keys() {
    let root = repository_root();
    let session_contract =
        fs::read_to_string(root.join("packages/d2b-contracts/src/v2_component_session.rs"))
            .expect("frozen ComponentSession contract");
    let service_inventory = fs::read_to_string(root.join("docs/reference/v2-services.json"))
        .expect("frozen service inventory");

    for component in COMPONENTS {
        if let Some(package) = component.service_package {
            assert!(
                service_inventory.contains(&format!(r#""package": "{package}""#)),
                "{} invents service package {package}",
                component.id
            );
            assert!(
                session_contract.contains(&format!("\"{package}\"")),
                "{} package is absent from ComponentSession",
                component.id
            );
        }
        if let Some(purpose) = component.endpoint_purpose {
            assert!(
                session_contract.contains(&format!("\"{purpose}\"")),
                "{} invents endpoint purpose {purpose}",
                component.id
            );
        }
        if let Some(role) = component.endpoint_role {
            assert!(
                session_contract.contains(&format!("\"{role}\"")),
                "{} invents endpoint role {role}",
                component.id
            );
        }
    }
}

#[test]
fn frozen_parent_blockers_are_external_to_local_ownership() {
    let root = repository_root();
    let blockers = BLOCKERS
        .iter()
        .map(|blocker| (blocker.id, blocker))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(blockers.len(), BLOCKERS.len(), "duplicate blocker id");

    for blocker in BLOCKERS {
        assert!(
            matches!(blocker.authority, "core-control-parent" | "shared-root"),
            "unknown parent authority for {}",
            blocker.id
        );
        for path in blocker.paths {
            assert!(root.join(path).is_file(), "blocker path {path} is missing");
            assert!(
                owner_for(path).is_empty(),
                "frozen parent path {path} has a local owner"
            );
        }
    }
    for component in COMPONENTS {
        for blocker in component.frozen_parent_blockers {
            assert!(
                blockers.contains_key(blocker),
                "{} names unknown blocker {blocker}",
                component.id
            );
        }
    }
}

#[test]
fn frozen_contract_dependencies_are_known_and_external() {
    let root = repository_root();
    let inputs = FROZEN_INPUTS
        .iter()
        .map(|input| (input.id, input))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        inputs.len(),
        FROZEN_INPUTS.len(),
        "duplicate frozen input id"
    );

    for input in FROZEN_INPUTS {
        for path in input.paths {
            assert!(root.join(path).is_file(), "frozen input {path} is missing");
            assert!(
                owner_for(path).is_empty(),
                "frozen input {path} has a local owner"
            );
        }
    }
    for component in COMPONENTS {
        for input in component.frozen_inputs {
            assert!(
                inputs.contains_key(input),
                "{} names unknown frozen input {input}",
                component.id
            );
        }
    }
}

#[test]
fn legacy_ipc_inventory_has_no_specialized_exception() {
    let root = repository_root();
    let components = COMPONENTS
        .iter()
        .map(|component| component.id)
        .collect::<BTreeSet<_>>();
    let blockers = BLOCKERS
        .iter()
        .map(|blocker| blocker.id)
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();

    for boundary in LEGACY_BOUNDARIES {
        assert!(
            ids.insert(boundary.id),
            "duplicate boundary {}",
            boundary.id
        );
        assert!(components.contains(boundary.owner));
        assert!(!boundary.legacy_handshake.trim().is_empty());
        assert!(
            boundary.disposition.contains("d2b.")
                || boundary.disposition == "fold-or-component-session"
                || boundary.disposition == "guest-service-or-delete"
                || boundary.disposition == "inherited-component-session",
            "{} has no migration or deletion disposition",
            boundary.id
        );
        if let Some(blocker) = boundary.frozen_parent_blocker {
            assert!(blockers.contains(blocker));
        }
        for path in boundary.call_graph {
            assert!(
                root.join(path).is_file(),
                "{} call graph misses {path}",
                boundary.id
            );
        }
    }

    let required = BTreeSet::from([
        "activation-argv-and-stdin",
        "clipboard-cli-control",
        "clipboard-picker-child",
        "clipboard-proxy-bridge",
        "guest-shell-parent-control",
        "notify-projection-actions",
        "runtime-daemon-helper",
        "runtime-scope-bootstrap",
        "security-key-report-stream",
        "shell-supervisor-bootstrap",
        "shell-supervisor-control",
        "tty-status-channel",
        "wayland-display-listener",
        "wayland-runtime-readiness",
    ]);
    assert_eq!(ids, required);
}
