use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
struct Component {
    id: &'static str,
    owned_files: &'static [&'static str],
    reserved_files: &'static [&'static str],
    reserved_prefixes: &'static [&'static str],
    reserved_test_prefixes: &'static [&'static str],
    implementation_dependencies: &'static [&'static str],
    final_composition_dependencies: &'static [&'static str],
    frozen_inputs: &'static [&'static str],
    service_module: Option<ServiceModule>,
    service_package: Option<&'static str>,
    endpoint_purpose: Option<&'static str>,
    endpoint_role: Option<&'static str>,
    frozen_parent_blockers: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct ServiceModule {
    path: &'static str,
    package_const: &'static str,
    purpose_const: &'static str,
    role_const: &'static str,
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

const PREP_INTEGRATOR_FILES: &[&str] = &[
    "CHANGELOG.md",
    "delivery/manifests/w6.json",
    "packages/d2b-userd/tests/edge_composition_policy.rs",
    "packages/d2b-userd/tests/edge_segment_preflight.rs",
];
const PREP_INTEGRATOR_ID: &str = "prep-integrator";
const PREP_INTEGRATOR_TEST_PREFIXES: &[&str] = &["packages/d2b-userd/tests/prep/"];
const FOREIGN_IMPLEMENTATION_PREFIXES: &[&str] = &[
    "examples/",
    "nixos-modules/",
    "pkgs/",
    "templates/",
    "tests/unit/nix/",
    "tests/unit/smoke/",
];
const DOCUMENTED_COMPONENTS: &[&str] = &[
    "activation-one-shot",
    "clipboard-bridge",
    "clipboard-control",
    "clipboard-picker",
    "desktop-actions",
    "desktop-observer",
    "retained-guest-shell",
    "runtime-systemd-user",
    "security-key-controller",
    "security-key-frontend",
    "shell-supervisor",
    "tty-one-shot",
    "user-secrets",
    "wayland-control",
];

const BLOCKERS: &[FrozenParentBlocker] = &[
    FrozenParentBlocker {
        id: "activation-bootstrap",
        authority: "core-control-parent",
        paths: &[
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
            "packages/d2b-priv-broker/src/ops/store_sync.rs",
            "packages/d2b-priv-broker/src/ops/store_verify.rs",
            "packages/d2b-priv-broker/src/ops/store_view_farm.rs",
            "packages/d2b-priv-broker/src/runtime.rs",
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-host/src/hardlink_farm.rs",
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
        id: "retained-host-helper-caller",
        authority: "declarative-host-parent",
        paths: &["nixos-modules/host-activation.nix"],
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
        owned_files: &[],
        reserved_files: &[
            "docs/how-to/use-activation-service.md",
            "docs/reference/activation-service.md",
        ],
        reserved_prefixes: &[
            "packages/d2b-activation-helper/",
            "packages/d2b-one-shot-helper/",
        ],
        reserved_test_prefixes: &[
            "packages/d2b-activation-helper/tests/activation/",
            "packages/d2b-one-shot-helper/tests/activation/",
        ],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &["component-session", "service-contracts"],
        service_module: None,
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["activation-bootstrap", "workspace-membership"],
    },
    Component {
        id: "clipboard-bridge",
        owned_files: &[
            "packages/d2b-clipd/src/fd.rs",
            "packages/d2b-wayland-proxy/src/bridge.rs",
            "packages/d2b-wayland-proxy/src/clipboard.rs",
        ],
        reserved_files: &["docs/reference/clipboard-bridge.md"],
        reserved_prefixes: &["packages/d2b-clipd/src/services/bridge/"],
        reserved_test_prefixes: &["packages/d2b-clipd/tests/bridge/"],
        implementation_dependencies: &["wayland-control"],
        final_composition_dependencies: &["clipboard-control"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-clipd/src/services/bridge/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
        service_package: Some("d2b.clipboard.v2"),
        endpoint_purpose: Some("clipboard-bridge"),
        endpoint_role: Some("clipboard-daemon"),
        frozen_parent_blockers: &[],
    },
    Component {
        id: "clipboard-composition",
        owned_files: &[
            "packages/d2b-clipd/Cargo.toml",
            "packages/d2b-clipd/src/daemon.rs",
            "packages/d2b-clipd/src/lib.rs",
            "packages/d2b-clipd/src/main.rs",
            "packages/d2b-clipd/src/services/mod.rs",
            "packages/d2b-clipd/tests/daemon_cli.rs",
        ],
        reserved_files: &[],
        reserved_prefixes: &[],
        reserved_test_prefixes: &["packages/d2b-clipd/tests/composition/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[
            "clipboard-bridge",
            "clipboard-control",
            "clipboard-picker",
        ],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: None,
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
            "docs/explanation/clipboard-architecture.md",
            "docs/reference/clipboard-policy.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-clipd/src/services/control/"],
        reserved_test_prefixes: &["packages/d2b-clipd/tests/control/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-clipd/src/services/control/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
            "docs/how-to/configure-clipboard-picker.md",
            "docs/reference/clipboard-picker-protocol.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-clipd/src/services/picker/"],
        reserved_test_prefixes: &["packages/d2b-clipd/tests/picker/"],
        implementation_dependencies: &["clipboard-control"],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-clipd/src/services/picker/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
        reserved_files: &[
            "docs/how-to/use-wlcontrol.md",
            "docs/reference/desktop-actions.md",
        ],
        reserved_prefixes: &[
            "packages/d2b-notify/src/services/actions/",
            "packages/d2b-wlcontrol/",
        ],
        reserved_test_prefixes: &[
            "packages/d2b-notify/tests/actions/",
            "packages/d2b-wlcontrol/tests/actions/",
        ],
        implementation_dependencies: &["desktop-observer"],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-notify/src/services/actions/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
        reserved_files: &[],
        reserved_prefixes: &[],
        reserved_test_prefixes: &["packages/d2b-notify/tests/composition/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &["desktop-actions", "desktop-observer"],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: None,
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
            "docs/reference/usb-security-key-events.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-notify/src/services/observer/"],
        reserved_test_prefixes: &["packages/d2b-notify/tests/observer/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-notify/src/services/observer/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
            "docs/reference/components-shell.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-guest-shell-runner/src/services/retained_shell/"],
        reserved_test_prefixes: &["packages/d2b-guest-shell-runner/tests/retained_shell/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &["component-session", "service-contracts"],
        service_module: Some(ServiceModule {
            path: "packages/d2b-guest-shell-runner/src/services/retained_shell/mod.rs",
            package_const: "PARENT_SERVICE_PACKAGE",
            purpose_const: "PARENT_ENDPOINT_PURPOSE",
            role_const: "PARENT_ENDPOINT_ROLE",
        }),
        service_package: Some("d2b.guest.v2"),
        endpoint_purpose: Some("guest-control"),
        endpoint_role: Some("guest-agent"),
        frozen_parent_blockers: &["guest-shell-bootstrap"],
    },
    Component {
        id: "retained-host-helper",
        owned_files: &[
            "packages/d2b-host-activation-helper/Cargo.toml",
            "packages/d2b-host-activation-helper/src/main.rs",
        ],
        reserved_files: &[],
        reserved_prefixes: &[],
        reserved_test_prefixes: &["packages/d2b-host-activation-helper/tests/retained/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[],
        service_module: None,
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
        frozen_parent_blockers: &["retained-host-helper-caller"],
    },
    Component {
        id: "runtime-composition",
        owned_files: &[
            "packages/d2b-unsafe-local-helper/Cargo.toml",
            "packages/d2b-unsafe-local-helper/src/lib.rs",
            "packages/d2b-unsafe-local-helper/src/main.rs",
            "packages/d2b-unsafe-local-helper/src/server.rs",
            "packages/d2b-unsafe-local-helper/src/services/mod.rs",
        ],
        reserved_files: &[],
        reserved_prefixes: &[],
        reserved_test_prefixes: &["packages/d2b-unsafe-local-helper/tests/composition/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[
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
        service_module: None,
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
            "docs/explanation/unsafe-local-runtime.md",
            "docs/how-to/configure-unsafe-local-launchers.md",
            "docs/reference/unsafe-local-provider.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &[
            "packages/d2b-runtime-systemd-user/",
            "packages/d2b-systemd-user-agent/",
            "packages/d2b-unsafe-local-helper/src/services/runtime_systemd_user/",
        ],
        reserved_test_prefixes: &[
            "packages/d2b-runtime-systemd-user/tests/runtime/",
            "packages/d2b-systemd-user-agent/tests/runtime/",
            "packages/d2b-unsafe-local-helper/tests/runtime_systemd_user/",
        ],
        implementation_dependencies: &["wayland-control"],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-unsafe-local-helper/src/services/runtime_systemd_user/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
        service_package: Some("d2b.runtime.systemd-user.v2"),
        endpoint_purpose: Some("runtime-systemd-user"),
        endpoint_role: Some("runtime-systemd-user-agent"),
        frozen_parent_blockers: &["runtime-endpoint", "workspace-membership"],
    },
    Component {
        id: "security-key-controller",
        owned_files: &[
            "docs/explanation/usb-security-key-architecture.md",
            "docs/how-to/migrate-usbip-yubikey-to-security-key.md",
            "docs/how-to/use-usb-security-key.md",
        ],
        reserved_files: &["docs/reference/security-key-service.md"],
        reserved_prefixes: &["packages/d2b-security-key-helper/"],
        reserved_test_prefixes: &["packages/d2b-security-key-helper/tests/controller/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "device-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_module: None,
        service_package: None,
        endpoint_purpose: None,
        endpoint_role: None,
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
            "docs/reference/components-usb-security-key.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-sk-frontend/src/services/security_key/"],
        reserved_test_prefixes: &["packages/d2b-sk-frontend/tests/frontend/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &["security-key-controller"],
        frozen_inputs: &[
            "component-session",
            "device-provider",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-sk-frontend/src/services/security_key/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
            "docs/explanation/persistent-shells.md",
            "docs/how-to/use-persistent-shells.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &[
            "packages/d2b-shell-supervisor/",
            "packages/d2b-unsafe-local-helper/src/services/shell/",
        ],
        reserved_test_prefixes: &[
            "packages/d2b-shell-supervisor/tests/shell/",
            "packages/d2b-unsafe-local-helper/tests/shell/",
        ],
        implementation_dependencies: &["runtime-systemd-user"],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "observability-result",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-unsafe-local-helper/src/services/shell/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
        service_package: Some("d2b.shell.v2"),
        endpoint_purpose: Some("shell-supervisor"),
        endpoint_role: Some("shell-supervisor"),
        frozen_parent_blockers: &["runtime-endpoint", "workspace-membership"],
    },
    Component {
        id: "tty-one-shot",
        owned_files: &["packages/d2b-unsafe-local-helper/src/tty_exec.rs"],
        reserved_files: &[
            "docs/how-to/use-tty-helper.md",
            "docs/reference/tty-service.md",
        ],
        reserved_prefixes: &[
            "packages/d2b-tty-helper/",
            "packages/d2b-unsafe-local-helper/src/services/tty/",
        ],
        reserved_test_prefixes: &[
            "packages/d2b-tty-helper/tests/tty/",
            "packages/d2b-unsafe-local-helper/tests/tty/",
        ],
        implementation_dependencies: &[],
        final_composition_dependencies: &["runtime-systemd-user"],
        frozen_inputs: &["component-session", "service-contracts"],
        service_module: Some(ServiceModule {
            path: "packages/d2b-unsafe-local-helper/src/services/tty/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
            "packages/d2b-userd/tests/fail_closed.rs",
        ],
        reserved_files: &[
            "docs/how-to/manage-user-secrets.md",
            "docs/reference/user-secrets-and-unattended-credentials.md",
        ],
        reserved_prefixes: &["packages/d2b-userd/src/services/user/"],
        reserved_test_prefixes: &["packages/d2b-userd/tests/user/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "credential-placement",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-userd/src/services/user/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
        reserved_files: &[],
        reserved_prefixes: &[],
        reserved_test_prefixes: &["packages/d2b-wayland-proxy/tests/composition/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &["clipboard-bridge", "wayland-control"],
        frozen_inputs: &[
            "component-session",
            "display-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_module: None,
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
            "docs/how-to/migrate-to-wayland-proxy.md",
            "docs/reference/wayland-proxy-warnings.md",
        ],
        reserved_files: &[],
        reserved_prefixes: &["packages/d2b-wayland-proxy/src/services/wayland/"],
        reserved_test_prefixes: &["packages/d2b-wayland-proxy/tests/wayland/"],
        implementation_dependencies: &[],
        final_composition_dependencies: &[],
        frozen_inputs: &[
            "component-session",
            "display-provider",
            "observability-result",
            "provider-framework",
            "service-contracts",
            "transport-contract",
        ],
        service_module: Some(ServiceModule {
            path: "packages/d2b-wayland-proxy/src/services/wayland/mod.rs",
            package_const: "SERVICE_PACKAGE",
            purpose_const: "ENDPOINT_PURPOSE",
            role_const: "ENDPOINT_ROLE",
        }),
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
            "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
            "packages/d2b-priv-broker/src/ops/store_sync.rs",
            "packages/d2b-priv-broker/src/ops/store_verify.rs",
            "packages/d2b-priv-broker/src/ops/store_view_farm.rs",
            "packages/d2b-priv-broker/src/runtime.rs",
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-host/src/hardlink_farm.rs",
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
        id: "retained-host-helper-argv",
        owner: "retained-host-helper",
        call_graph: &[
            "nixos-modules/host-activation.nix",
            "packages/d2b-host-activation-helper/src/main.rs",
        ],
        legacy_handshake: "host activation invokes fixed chgrp-by-numeric-gid argv and consumes process status",
        disposition: "migrate-or-delete",
        frozen_parent_blocker: Some("retained-host-helper-caller"),
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
    let mut owners = Vec::new();
    if PREP_INTEGRATOR_FILES.contains(&path) {
        owners.push(PREP_INTEGRATOR_ID);
    }
    if PREP_INTEGRATOR_TEST_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        owners.push(PREP_INTEGRATOR_ID);
    }
    owners.extend(
        COMPONENTS
            .iter()
            .filter(|component| {
                component.owned_files.contains(&path)
                    || component.reserved_files.contains(&path)
                    || component
                        .reserved_prefixes
                        .iter()
                        .any(|prefix| path.starts_with(prefix))
                    || component
                        .reserved_test_prefixes
                        .iter()
                        .any(|prefix| path.starts_with(prefix))
            })
            .map(|component| component.id),
    );
    owners
}

#[test]
fn edge_files_docs_and_test_reservations_have_exactly_one_owner() {
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
        assert!(
            !component.reserved_test_prefixes.is_empty(),
            "{} must reserve a disjoint future test path",
            component.id
        );
        for file in component.owned_files {
            assert!(root.join(file).is_file(), "{} is missing", file);
            assert_eq!(owner_for(file), vec![component.id]);
        }
        for file in component.reserved_files {
            assert_eq!(owner_for(file), vec![component.id]);
        }
        for prefix in component
            .reserved_prefixes
            .iter()
            .chain(component.reserved_test_prefixes)
        {
            assert!(prefix.ends_with('/'), "reserved prefix must end in /");
            let probe = format!("{prefix}__reserved__");
            assert_eq!(owner_for(&probe), vec![component.id]);
        }
    }
    let components = COMPONENTS
        .iter()
        .map(|component| (component.id, component))
        .collect::<BTreeMap<_, _>>();
    for id in DOCUMENTED_COMPONENTS {
        let component = components[id];
        assert!(
            component
                .owned_files
                .iter()
                .chain(component.reserved_files)
                .any(|path| path.starts_with("docs/")),
            "{id} must own or reserve its W6 documentation"
        );
    }
    for file in PREP_INTEGRATOR_FILES {
        assert!(root.join(file).is_file(), "{} is missing", file);
        assert_eq!(owner_for(file), vec![PREP_INTEGRATOR_ID]);
    }
    for prefix in PREP_INTEGRATOR_TEST_PREFIXES {
        assert_eq!(
            owner_for(&format!("{prefix}__reserved__")),
            vec![PREP_INTEGRATOR_ID]
        );
    }
    for prefix in FOREIGN_IMPLEMENTATION_PREFIXES {
        assert!(
            owner_for(&format!("{prefix}reserved")).is_empty(),
            "foreign path prefix {prefix} has a local owner"
        );
    }
}

#[test]
fn implementation_and_final_composition_dependencies_are_known_and_acyclic() {
    let components = COMPONENTS
        .iter()
        .map(|component| (component.id, component))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(components.len(), COMPONENTS.len(), "duplicate component id");

    fn visit(
        id: &'static str,
        components: &BTreeMap<&'static str, &Component>,
        selector: fn(&Component) -> &'static [&'static str],
        active: &mut BTreeSet<&'static str>,
        complete: &mut BTreeSet<&'static str>,
    ) {
        if complete.contains(id) {
            return;
        }
        assert!(active.insert(id), "component dependency cycle at {id}");
        let component = components.get(id).expect("known component");
        for dependency in selector(component) {
            assert!(
                components.contains_key(dependency),
                "{id} has unknown dependency {dependency}"
            );
            visit(dependency, components, selector, active, complete);
        }
        active.remove(id);
        complete.insert(id);
    }

    for selector in [
        (|component: &Component| component.implementation_dependencies)
            as fn(&Component) -> &'static [&'static str],
        (|component: &Component| component.final_composition_dependencies)
            as fn(&Component) -> &'static [&'static str],
    ] {
        let mut complete = BTreeSet::new();
        for id in components.keys().copied() {
            visit(
                id,
                &components,
                selector,
                &mut BTreeSet::new(),
                &mut complete,
            );
        }
    }

    assert_eq!(
        components["clipboard-bridge"].implementation_dependencies,
        ["wayland-control"]
    );
    assert_eq!(
        components["runtime-systemd-user"].implementation_dependencies,
        ["wayland-control"]
    );
    assert!(
        components["tty-one-shot"]
            .implementation_dependencies
            .is_empty()
    );
    assert_eq!(
        components["tty-one-shot"].final_composition_dependencies,
        ["runtime-systemd-user"]
    );
}

fn public_string_const(root: &Path, module: &ServiceModule, name: &str) -> String {
    let source = fs::read_to_string(root.join(module.path)).expect("read service module");
    let prefix = format!("pub const {name}: &str = ");
    let declarations = source
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect::<Vec<_>>();
    assert_eq!(
        declarations.len(),
        1,
        "{} must define exactly one public {name}",
        module.path
    );
    let literal = declarations[0]
        .strip_suffix(';')
        .expect("public string constant ends with semicolon")
        .trim();
    let value = literal
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .expect("public string constant is a literal");
    assert!(
        !value.contains('\\') && !value.contains('"'),
        "service composition constant must be a plain literal"
    );
    value.to_owned()
}

#[test]
fn actual_service_modules_match_frozen_contract_keys() {
    let root = repository_root();
    let session_contract =
        fs::read_to_string(root.join("packages/d2b-contracts/src/v2_component_session.rs"))
            .expect("frozen ComponentSession contract");
    let service_inventory = fs::read_to_string(root.join("docs/reference/v2-services.json"))
        .expect("frozen service inventory");
    let mut assigned_modules = BTreeSet::new();

    for component in COMPONENTS {
        let Some(module) = component.service_module else {
            assert!(
                component.service_package.is_none()
                    && component.endpoint_purpose.is_none()
                    && component.endpoint_role.is_none(),
                "{} copies service keys without an actual module",
                component.id
            );
            continue;
        };
        assert!(
            assigned_modules.insert(module.path),
            "service module {} has multiple owners",
            module.path
        );
        assert_eq!(owner_for(module.path), vec![component.id]);

        let package = public_string_const(&root, &module, module.package_const);
        let purpose = public_string_const(&root, &module, module.purpose_const);
        let role = public_string_const(&root, &module, module.role_const);
        assert_eq!(Some(package.as_str()), component.service_package);
        assert_eq!(Some(purpose.as_str()), component.endpoint_purpose);
        assert_eq!(Some(role.as_str()), component.endpoint_role);
        assert!(
            service_inventory.contains(&format!(r#""package": "{package}""#)),
            "{} actual module invents service package {package}",
            component.id
        );
        for (kind, value) in [("package", package), ("purpose", purpose), ("role", role)] {
            assert!(
                session_contract.contains(&format!("\"{value}\"")),
                "{} actual {kind} {value} is absent from ComponentSession",
                component.id
            );
        }
    }

    let mut files = BTreeSet::new();
    for prefix in OWNED_PACKAGE_PREFIXES {
        collect_files(&root, Path::new(prefix), &mut files);
    }
    let discovered_modules = files
        .into_iter()
        .filter(|path| path.contains("/src/services/") && path.ends_with("/mod.rs"))
        .filter(|path| {
            let source =
                fs::read_to_string(root.join(path)).expect("read discovered service module");
            source.contains("pub const SERVICE_PACKAGE: &str")
                || source.contains("pub const PARENT_SERVICE_PACKAGE: &str")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        discovered_modules,
        assigned_modules
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        "every public service composition module must be parsed and owned"
    );
}

#[test]
fn frozen_parent_blockers_are_external_to_local_ownership() {
    let root = repository_root();
    let blockers = BLOCKERS
        .iter()
        .map(|blocker| (blocker.id, blocker))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(blockers.len(), BLOCKERS.len(), "duplicate blocker id");
    assert_eq!(
        blockers["activation-bootstrap"].paths,
        [
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
            "packages/d2b-priv-broker/src/ops/store_sync.rs",
            "packages/d2b-priv-broker/src/ops/store_verify.rs",
            "packages/d2b-priv-broker/src/ops/store_view_farm.rs",
            "packages/d2b-priv-broker/src/runtime.rs",
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-host/src/hardlink_farm.rs",
        ]
    );

    for blocker in BLOCKERS {
        assert!(
            matches!(
                blocker.authority,
                "core-control-parent" | "declarative-host-parent" | "shared-root"
            ),
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
fn final_parent_composition_uses_authenticated_fixed_endpoints() {
    let root = repository_root();
    let read = |path: &str| fs::read_to_string(root.join(path)).expect("read composition seam");

    let session_server = read("packages/d2b-session/src/server.rs");
    assert!(session_server.contains("serve_ttrpc_services"));
    let activation = read("packages/d2b-session-unix/src/systemd.rs");
    assert!(activation.contains("ActivatedSeqpacketListener"));
    assert!(activation.contains("ActivatedSeqpacketListeners"));

    let user_services = read("nixos-modules/user-services.nix");
    assert!(user_services.contains("/run/d2b/u/%U/userd.sock"));
    assert!(user_services.contains("ListenSequentialPacket"));
    let runtime = read("nixos-modules/unsafe-local-helper.nix");
    assert!(runtime.contains("/run/d2b/u/%U/runtime-agent.sock"));
    assert!(!runtime.contains("/run/d2b/unsafe-local-helper.sock"));
    let clipboard = read("nixos-modules/clipboard.nix");
    for endpoint in ["control.sock", "picker.sock", "bridge.sock"] {
        assert!(
            clipboard.contains(endpoint),
            "missing clipboard endpoint {endpoint}"
        );
    }
    assert!(!clipboard.contains("clipd.sock"));

    let host_daemon = read("nixos-modules/host-daemon.nix");
    assert!(!host_daemon.contains("unsafeLocalHelperSocketPath"));
    assert!(!host_daemon.contains("unsafeLocalHelperSocketGroup"));
    let cli = read("packages/d2b/src/lib.rs");
    assert!(!cli.contains("d2b-clipd/clipd.sock"));
    assert!(!cli.contains(r#"{\"type\":\"arm\"}"#));

    let wayland_argv = read("packages/d2b-host/src/wayland_proxy_argv.rs");
    assert!(wayland_argv.contains("--session-generation"));
    assert!(!wayland_argv.contains("--listen-socket"));
    assert!(!wayland_argv.contains("--upstream-socket"));
    let client = read("packages/d2b-client/src/host_socket.rs");
    for service in [
        "ServiceKind::User",
        "ServiceKind::RuntimeSystemdUser",
        "ServiceKind::Clipboard",
        "ServiceKind::Notify",
        "ServiceKind::SecurityKey",
        "ServiceKind::Wayland",
    ] {
        assert!(
            client.contains(service),
            "missing authenticated client seam {service}"
        );
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

fn validate_owned_segment(paths: &[String]) -> Result<(), String> {
    for path in paths {
        if owner_for(path).len() != 1 {
            return Err(format!("W6 segment contains unowned path {path}"));
        }
    }
    Ok(())
}

#[test]
fn ownership_examples_cover_code_tests_docs_and_foreign_examples() {
    assert_eq!(
        owner_for("packages/d2b-userd/tests/user/future_service.rs"),
        vec!["user-secrets"]
    );
    assert_eq!(
        owner_for("docs/reference/clipboard-policy.md"),
        vec!["clipboard-control"]
    );
    assert!(
        owner_for("examples/graphics-workstation/configuration.nix").is_empty(),
        "examples remain foreign declarative-host ownership"
    );
}

#[derive(Clone, Copy)]
struct StackFixtureNode {
    branch: &'static str,
    base_oid: &'static str,
    head_oid: &'static str,
}

fn historical_segment_for(
    nodes: &[StackFixtureNode],
    branch: &str,
) -> Option<(&'static str, &'static str)> {
    nodes
        .iter()
        .find(|node| node.branch == branch)
        .map(|node| (node.base_oid, node.head_oid))
}

#[test]
fn linearized_stack_excludes_w5_foreign_paths_from_w6_segment() {
    let w5_foreign = "packages/d2bd/src/lib.rs".to_owned();
    let w6_owned = "packages/d2b-userd/src/lib.rs".to_owned();
    assert!(owner_for(&w5_foreign).is_empty());
    assert!(validate_owned_segment(std::slice::from_ref(&w6_owned)).is_ok());
    assert!(
        validate_owned_segment(&[w5_foreign, w6_owned]).is_err(),
        "a shared-root-to-head diff would incorrectly include W5 history"
    );

    let graph = [
        StackFixtureNode {
            branch: "adr0045-w5-control",
            base_oid: "root",
            head_oid: "w5",
        },
        StackFixtureNode {
            branch: "adr0045-w6-edge",
            base_oid: "w5",
            head_oid: "w6",
        },
        StackFixtureNode {
            branch: "adr0045-w7-realm-host",
            base_oid: "w6",
            head_oid: "w7",
        },
    ];
    assert_eq!(
        historical_segment_for(&graph, "adr0045-w6-edge"),
        Some(("w5", "w6"))
    );
    assert_eq!(
        historical_segment_for(&graph, "adr0045-w7-realm-host"),
        Some(("w6", "w7"))
    );
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
                || boundary.disposition == "inherited-component-session"
                || boundary.disposition == "migrate-or-delete",
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
        "retained-host-helper-argv",
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
    assert_eq!(
        LEGACY_BOUNDARIES
            .iter()
            .find(|boundary| boundary.id == "activation-argv-and-stdin")
            .expect("activation boundary")
            .call_graph,
        [
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
            "packages/d2b-priv-broker/src/ops/store_sync.rs",
            "packages/d2b-priv-broker/src/ops/store_verify.rs",
            "packages/d2b-priv-broker/src/ops/store_view_farm.rs",
            "packages/d2b-priv-broker/src/runtime.rs",
            "packages/d2b-host/src/bin/d2b-activation-helper.rs",
            "packages/d2b-host/src/hardlink_farm.rs",
        ]
    );
}
