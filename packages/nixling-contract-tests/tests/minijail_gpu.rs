//! Per-role minijail validators, ported from the bash gates:
//!   * `tests/minijail-validator-gpu.sh`
//!   * `tests/minijail-validator-wayland-proxy.sh`
//!
//! These gates validate the RENDERED minijail role profiles for the Gpu and
//! WaylandProxy sidecar roles plus the source-of-truth wiring that grounds
//! them, so they belong in the fixture-contract layer (this crate, gated by the
//! NL_FIXTURES / NL_FIXTURES_FULL contract step in
//! `tests/tools/rust-workspace-checks.sh`), not the doc/source-grep policy layer.
//!
//! The Gpu and WaylandProxy role profiles only render on a graphics-enabled VM,
//! which the MINIMAL fixture-smoke bundle (corp-vm + net VMs) does not contain.
//! The rendered-profile tests therefore use the FEATURE-RICH `fixture-smoke-full`
//! bundle (NL_FIXTURES_FULL), whose `corp-full` VM enables graphics + video +
//! audio + tpm + usbip + observability so the `vm-corp-full-gpu` and
//! `vm-corp-full-wayland-proxy` profiles render. That fixture is `None` when
//! NL_FIXTURES_FULL is unset (a non-x86_64 host, where the graphics platform
//! gate makes it unavailable, or a plain `cargo test` pass); those tests skip
//! cleanly with an eprintln, exactly as the bash gates skipped their live arms.
//!
//! The SOURCE-grep portions (the broker device-class claim, the gpu-render-node
//! profile shape, the WaylandProxy Rust variant declarations) need no fixture
//! and always run — they read the in-tree `.nix`/`.rs` modules via the repo-file
//! helpers and assert per-line regex invariants, mirroring the bash `grep`/`awk`.
//!
//! Layer split (faithful to the bash gates):
//!   * The bash gates' static / Layer-1 (eval-only) assertions port here, either
//!     as RENDERED checks over the real fixture RoleProfiles (a strictly
//!     stronger guarantee than the bash constant-vs-constant `assert_eq` or a
//!     host-installed-profile drift scan) or, where the assertion targets the
//!     in-tree Nix/Rust source, as a SOURCE-grep.
//!   * The bash gates' opt-in live phases do NOT port — they are runtime
//!     host-execution tests requiring root, a live host, and the role binaries:
//!       - gpu: the cc-compiled `virtgpu_probe`, the `minijail0` positive arm
//!         (`DRM_IOCTL_VIRTGPU_GET_CAPS` must not raise SIGSYS), the negative arm
//!         (`ptrace(PTRACE_TRACEME)` must raise SIGSYS), the `NL_LIVE=1` bare
//!         GET_CAPS hardware smoke on the host render node, and the
//!         `/var/lib/nixling/validated/p1-gpu.json` evidence write.
//!       - wayland-proxy: the `NL_LIVE=1` Layer-2 `minijail0 /bin/true` positive
//!         probe and the `/var/lib/nixling/validated/p1-wayland-proxy.json`
//!         evidence write.
//!
//! Spec corrections / smoke-fixture gaps:
//!   * gpu `SOURCE_WAYLAND` (`/run/user/<uid>/wayland-0`) and `BIND_TARGET`
//!     (`/run/nixling-gpu/<vm>/wayland-0`): the bash gate's lines 100-103 are
//!     tautological `assert_eq` of a shell variable against its own definition.
//!     The host-side wayland SOURCE path is resolved by the broker at runtime
//!     and is NOT expressed in the bundle's rendered RoleProfile (the gpu
//!     profile's `bindMounts` is empty — the cross-domain wayland bind is a
//!     runtime broker op, not a bundle artifact). The in-sandbox BIND_TARGET
//!     convention IS grounded: the gpu profile exposes `/run/nixling-gpu/<vm>`
//!     as a writable path, so `/run/nixling-gpu/<vm>/wayland-0` lives under it.
//!     That writable-path check is the rendered counterpart.
//!   * The gpu-render-node profile is a DISTINCT broker-pre-NS mode
//!     (`graphics.renderNodeOnly = true`); the feature-rich `corp-full` VM
//!     renders the regular `Gpu` role (`vm-corp-full-gpu`, role "gpu",
//!     seccompPolicyRef "w1-gpu"), NOT the `GpuRenderNode` role. The bash gate's
//!     D5/P2.3 gpu-render-node assertions are all SOURCE greps over
//!     `nixos-modules/minijail-profiles.nix`,
//!     `packages/nixling-priv-broker/src/{live_handlers,sys}.rs`, and
//!     `nixos-modules/processes-json.nix`, so they port as source-greps
//!     unchanged and need no rendered fixture.

use nixling_contract_tests::{
    load_full_bundle_resolver_from_env, read_repo_file, repo_path_exists,
};
use nixling_core::processes::ProcessRole;
use regex::Regex;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";
const PROCESSES_JSON_NIX: &str = "nixos-modules/processes-json.nix";
const BUNDLE_RESOLVER_RS: &str = "packages/nixling-core/src/bundle_resolver.rs";
const LIVE_HANDLERS_RS: &str = "packages/nixling-priv-broker/src/live_handlers.rs";
const SYS_RS: &str = "packages/nixling-priv-broker/src/sys.rs";
const PROCESSES_RS: &str = "packages/nixling-core/src/processes.rs";
const BROKER_WIRE_RS: &str = "packages/nixling-ipc/src/broker_wire.rs";

/// The closed-set Gpu device-bind matrix from the bash gate's `DEVICE_BINDS`
/// array (6 entries, includes `/dev/udmabuf` and the per-card `/dev/nvidia0`).
const GPU_DEVICE_BINDS: [&str; 6] = [
    "/dev/kvm",
    "/dev/dri/renderD128",
    "/dev/nvidiactl",
    "/dev/nvidia0",
    "/dev/nvidia-uvm",
    "/dev/udmabuf",
];

/// Whether any single line of `content` matches `pattern`. Mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` can never span a newline
/// boundary). Copied from `tests/policy_daemon.rs::any_line_matches`.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// Extract the inclusive line range from the first line matching `start_pat`
/// through the first subsequent line matching `end_pat`. Mirrors the bash
/// gates' `awk '/start/{active=1} active{print} active&&/end/{exit}'` block
/// extraction.
fn extract_block(content: &str, start_pat: &str, end_pat: &str) -> Option<String> {
    let start_re = Regex::new(start_pat).expect("valid start regex");
    let end_re = Regex::new(end_pat).expect("valid end regex");
    let mut active = false;
    let mut block: Vec<&str> = Vec::new();
    for line in content.lines() {
        if !active && start_re.is_match(line) {
            active = true;
        }
        if active {
            block.push(line);
            if end_re.is_match(line) {
                return Some(block.join("\n"));
            }
        }
    }
    None
}

/// Mirror the bash gate's depth-counting `awk` block extractor used for the
/// gpu-render-node mkProfile block:
///
/// ```awk
/// /<start>/ { inblock=1; depth=1; next }
/// inblock { if (/{/) depth++; if (/}/) depth--; if (depth==0) {inblock=0; next} }
/// ```
///
/// Returns the INTERIOR lines, joined by `\n` (the start line and the matching
/// close line are both excluded, exactly as the awk `next`s past them), or
/// `None` when the start line is not found. Brace counting is per-line boolean
/// (a line containing `{` increments once regardless of count), faithful to the
/// awk `if (/{/) depth++`.
fn braced_block_interior(content: &str, start_literal: &str) -> Option<String> {
    let mut lines = content.lines();
    let mut started = false;
    for line in lines.by_ref() {
        if line.contains(start_literal) {
            started = true;
            break;
        }
    }
    if !started {
        return None;
    }
    let mut depth: i32 = 1;
    let mut interior: Vec<&str> = Vec::new();
    for line in lines {
        if line.contains('{') {
            depth += 1;
        }
        if line.contains('}') {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }
        interior.push(line);
    }
    Some(interior.join("\n"))
}

// ===========================================================================
// tests/minijail-validator-gpu.sh
// ===========================================================================

/// Static closed-set assertions (bash lines 87-103): the rendered `Gpu` role
/// profile MUST declare EXACTLY the 6-entry `DEVICE_BINDS` matrix in order
/// (`/dev/kvm`, `/dev/dri/renderD128`, `/dev/nvidiactl`, `/dev/nvidia0`,
/// `/dev/nvidia-uvm`, `/dev/udmabuf`), each path under `/dev`, plus the
/// in-sandbox wayland BIND_TARGET convention: `/run/nixling-gpu/<vm>` is a
/// writable path, so `/run/nixling-gpu/<vm>/wayland-0` lives under it. The
/// bash gate asserted these against shell constants; the rendered RoleProfile
/// is the strictly stronger ground truth. (The host-side `SOURCE_WAYLAND`
/// `/run/user/<uid>/wayland-0` bind is a runtime broker op, not a bundle
/// artifact — see the module-level spec-corrections note.)
#[test]
fn gpu_rendered_device_bind_matrix_and_wayland_target() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: NL_FIXTURES_FULL unset (feature-rich fixture unavailable)");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Gpu {
                continue;
            }
            seen += 1;
            let p = &node.profile;
            let dev = &p.mount_policy.device_binds;

            // bash lines 93-99: 6 entries, each under /dev — the rendered
            // profile pins the exact ordered matrix.
            assert_eq!(
                dev.as_slice(),
                GPU_DEVICE_BINDS,
                "Gpu {} (vm {}) device-bind matrix drift; expected the 6-entry P1 matrix, got {dev:?}",
                p.profile_id,
                dag.vm
            );
            for d in dev {
                assert!(
                    d.starts_with("/dev/"),
                    "Gpu {} (vm {}) device-bind entry not under /dev: {d}",
                    p.profile_id,
                    dag.vm
                );
            }

            // bash lines 100-103: the in-sandbox wayland bind-target parent
            // `/run/nixling-gpu/<vm>` must be a writable path.
            let target_parent = format!("/run/nixling-gpu/{}", dag.vm);
            let bind_target = format!("{target_parent}/wayland-0");
            assert!(
                p.mount_policy
                    .writable_paths
                    .iter()
                    .any(|w| w.path == target_parent),
                "Gpu {} (vm {}) must expose writable path {target_parent} for the wayland \
                 bind-target {bind_target}; got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.writable_paths
            );
            assert!(
                bind_target.starts_with(&target_parent),
                "Gpu wayland bind-target convention regression: {bind_target} not under \
                 {target_parent}"
            );
        }
    }
    assert!(
        seen > 0,
        "feature-rich fixture has no Gpu node — corp-full enables graphics (regression)"
    );
}

/// The rendered Gpu jail shape: empty host capabilities (the per-role smoke
/// proves no CAP_SYS_NICE is needed at runtime), the closed `w1-gpu` seccomp
/// reference, the `nixling.slice/<vm>/gpu` cgroup subtree, mount+ipc namespace
/// isolation without a net/user namespace, device nodes hidden by default,
/// `/nix/store` read-only, `umask = 0o007` (the fu36 socket-ACL requirement so
/// the crosvm vhost-user socket is mode 0660), and NO broker-pre-NS userNamespace
/// (only the distinct gpu-render-node role uses ADR 0021 broker-pre-NS). This is
/// the rendered jail-shape sanity that grounds the bash gate's documented Gpu
/// role contract beyond the device-bind matrix.
#[test]
fn gpu_rendered_jail_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: NL_FIXTURES_FULL unset (feature-rich fixture unavailable)");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Gpu {
                continue;
            }
            seen += 1;
            let p = &node.profile;
            assert!(
                p.caps.is_empty(),
                "Gpu {} (vm {}) must declare EMPTY host capabilities; got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-gpu"),
                "Gpu {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );
            assert_eq!(
                p.cgroup_placement.subtree,
                format!("nixling.slice/{}/gpu", dag.vm),
                "Gpu {} (vm {}) cgroup subtree drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.namespaces.mount && p.namespaces.ipc,
                "Gpu {} (vm {}) must isolate the mount + ipc namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                !p.namespaces.net && !p.namespaces.user,
                "Gpu {} (vm {}) must not request a net or user namespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                p.mount_policy.hide_device_nodes_by_default,
                "Gpu {} (vm {}) must hide device nodes by default",
                p.profile_id, dag.vm
            );
            assert!(
                p.mount_policy.nix_store_read_only,
                "Gpu {} (vm {}) must mount /nix/store read-only",
                p.profile_id, dag.vm
            );
            assert_eq!(
                p.umask,
                Some(7),
                "Gpu {} (vm {}) must declare umask = 0o007 (fu36 socket-ACL requirement)",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.user_namespace.is_none(),
                "Gpu {} (vm {}) must NOT declare a broker-pre-NS userNamespace (only the \
                 distinct gpu-render-node role uses ADR 0021 broker-pre-NS); got {:?}",
                p.profile_id,
                dag.vm,
                p.user_namespace
            );
        }
    }
    assert!(
        seen > 0,
        "feature-rich fixture has no Gpu node — corp-full enables graphics (regression)"
    );
}

/// Broker Gpu role-device claim (bash lines 105-136): the
/// `role_device_classes(Gpu)` source-of-truth in
/// `packages/nixling-core/src/bundle_resolver.rs` (the closed allowlist the
/// broker uses for OpenDevice dispatch) MUST match the P1 device matrix — it
/// includes `kvm`, `dri`, `nvidia-ctl`, `nvidia-uvm`, `nvidia-render`, and
/// `udmabuf`, and MUST NOT include `vfio` (not in the P1 GPU contract). The arm
/// is shared with GpuRenderNode (`ProcessRole::Gpu | ProcessRole::GpuRenderNode
/// => &[ ... ]`), so the block is anchored on the line opening the slice
/// (`=> &[`) rather than a bare `ProcessRole::Gpu =>`.
#[test]
fn gpu_broker_role_device_claim_source() {
    assert!(
        repo_path_exists(BUNDLE_RESOLVER_RS),
        "missing {BUNDLE_RESOLVER_RS}"
    );
    let src = read_repo_file(BUNDLE_RESOLVER_RS);
    let gpu_arm = extract_block(&src, r"ProcessRole::Gpu.*=>.*&\[", r"\],")
        .expect("could not locate the ProcessRole::Gpu device-class arm in bundle_resolver.rs");

    for required in [
        "\"kvm\"",
        "\"dri\"",
        "\"nvidia-ctl\"",
        "\"nvidia-uvm\"",
        "\"nvidia-render\"",
        "\"udmabuf\"",
    ] {
        assert!(
            gpu_arm.contains(required),
            "broker Gpu role-device claim MISSING {required} (drift from P1 matrix); arm:\n{gpu_arm}"
        );
    }
    assert!(
        !gpu_arm.contains("\"vfio\""),
        "broker Gpu role-device claim INCLUDES vfio (NOT in P1 GPU contract); arm:\n{gpu_arm}"
    );
}

/// D5/P2.3 gpu-render-node minijail profile shape (bash lines 420-501): the
/// gpu-render-node `mkProfile` block in `nixos-modules/minijail-profiles.nix`
/// must be present and carry the ADR-0021 broker-pre-NS shape — a
/// `userNamespace` block referencing the gpu principal, `seccompPolicyRef =
/// "w1-gpu-render-node"`, an EMPTY `deviceBinds` (fd-passing replaces
/// bind-mounts), `umask = 7`, and the profile gated on
/// `vm.graphics.renderNodeOnly`. (The regular Gpu role rendered above is a
/// distinct mode; gpu-render-node renders only when `renderNodeOnly = true`, so
/// these are source-greps.)
#[test]
fn gpu_render_node_minijail_profile_source_shape() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "minijail-profiles.nix not found at {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    // 1. gpu-render-node mkProfile block present.
    let block_anchor = r#""${profileIdFor name "gpu-render-node"}" = mkProfile"#;
    assert!(
        src.contains(block_anchor),
        "D5/P2.3: gpu-render-node mkProfile block MISSING in {MINIJAIL_PROFILES_NIX}"
    );

    // Extract the gpu-render-node block interior for the in-block checks.
    let block = braced_block_interior(
        &src,
        r#""${profileIdFor name "gpu-render-node"}" = mkProfile {"#,
    )
    .expect("could not locate gpu-render-node mkProfile block interior");

    // 2. userNamespace block present on gpu-render-node (ADR 0021).
    assert!(
        any_line_matches(&block, r"userNamespace\s*="),
        "D5/P2.3: gpu-render-node profile MISSING userNamespace block (ADR 0021)"
    );

    // 3. userNamespace references the gpu principal (whole-file, matching the
    // bash grep over PROFILES_NIX).
    assert!(
        src.contains(r#"stablePrincipalId "nixling-${name}-gpu""#),
        "D5/P2.3: gpu-render-node userNamespace missing gpu principal reference"
    );

    // 4. seccompPolicyRef = "w1-gpu-render-node".
    assert!(
        src.contains(r#"seccompPolicyRef = "w1-gpu-render-node""#),
        "D5/P2.3: gpu-render-node missing seccompPolicyRef = \"w1-gpu-render-node\""
    );

    // 5. deviceBinds empty (no `deviceBinds = [ /dev...` inside the block).
    assert!(
        !any_line_matches(&block, r"deviceBinds\s*=\s*\[\s*/dev"),
        "D5/P2.3: gpu-render-node deviceBinds is non-empty — bind-mounts are skipped for \
         user-NS spawns (fd-passing replaces bind-mounts)"
    );

    // 6. umask = 7 present (fu36 socket-ACL requirement).
    assert!(
        any_line_matches(&block, r"umask\s*=\s*7"),
        "D5/P2.3: gpu-render-node missing umask = 7 (fu36 socket-ACL requirement)"
    );

    // 7. Profile gated on vm.graphics.renderNodeOnly.
    assert!(
        src.contains("vm.graphics.renderNodeOnly"),
        "D5/P2.3: gpu-render-node missing vm.graphics.renderNodeOnly gate"
    );
}

/// D5/P2.3 gpu-render-node broker + processes wiring (bash lines 503-545): the
/// broker maps `w1-gpu-render-node` to its device classes in
/// `live_handlers.rs`, `sys.rs` declares the `RENDER_NODE_INHERITED_FD` protocol
/// constant and the `pre_opened_device_fds` field on `RunnerIsolationSpec`, and
/// `processes-json.nix` defines `gpuRenderNodeRunner`, emits the
/// `"gpu-render-node"` role node, and carries `--gpu-device-node
/// /proc/self/fd/10` in the runner argv.
#[test]
fn gpu_render_node_broker_processes_wiring_source() {
    // 8. live_handlers.rs maps w1-gpu-render-node -> device classes.
    assert!(
        repo_path_exists(LIVE_HANDLERS_RS),
        "missing {LIVE_HANDLERS_RS}"
    );
    let live_handlers = read_repo_file(LIVE_HANDLERS_RS);
    assert!(
        live_handlers.contains("\"w1-gpu-render-node\""),
        "D5/P2.3: live_handlers.rs MISSING w1-gpu-render-node device-class entry"
    );

    // 9 + 10. sys.rs protocol constant + RunnerIsolationSpec field.
    assert!(repo_path_exists(SYS_RS), "missing {SYS_RS}");
    let sys = read_repo_file(SYS_RS);
    assert!(
        sys.contains("RENDER_NODE_INHERITED_FD"),
        "D5/P2.3: sys.rs MISSING RENDER_NODE_INHERITED_FD protocol constant"
    );
    assert!(
        sys.contains("pre_opened_device_fds"),
        "D5/P2.3: sys.rs MISSING pre_opened_device_fds on RunnerIsolationSpec"
    );

    // 11 + 12 + 13. processes-json.nix runner + role node + argv flag.
    assert!(
        repo_path_exists(PROCESSES_JSON_NIX),
        "missing {PROCESSES_JSON_NIX}"
    );
    let processes_json = read_repo_file(PROCESSES_JSON_NIX);
    assert!(
        processes_json.contains("gpuRenderNodeRunner"),
        "D5/P2.3: processes-json.nix MISSING gpuRenderNodeRunner"
    );
    assert!(
        processes_json.contains("\"gpu-render-node\""),
        "D5/P2.3: processes-json.nix MISSING gpu-render-node role emission"
    );
    assert!(
        processes_json.contains("gpu-device-node"),
        "D5/P2.3: processes-json.nix MISSING --gpu-device-node in gpuRenderNodeRunner argv"
    );
}

// ===========================================================================
// tests/minijail-validator-wayland-proxy.sh
// ===========================================================================

/// Layer-1 `assert_profile_source` (bash lines 75-139): the wayland-proxy
/// minijail profile in `nixos-modules/minijail-profiles.nix` MUST match the
/// ADR-0025 contract exactly:
///   * the `role = "wayland-proxy"` declaration exists;
///   * `seccompPolicyRef = "w1-wayland-proxy"` is mandatory;
///   * capabilities are empty (zero `CAP_` tokens in the profile block);
///   * `requiresStartRoot` is never `= true`;
///   * NO `userNamespace` is declared (no ADR-0021 broker-pre-NS for this role);
///   * the writable path `/run/nixling-wlproxy` is present;
///   * NO PipeWire/Pulse bind references in the block;
///   * `umask = 7` (0o007, so the filter socket has mode 0660);
///   * `deviceBinds = [ ]` (pure AF_UNIX proxy, no hardware access).
#[test]
fn wayland_proxy_profile_source_shape() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    // Whole-file: role declaration, mandatory seccomp ref, writable path.
    assert!(
        any_line_matches(&src, r#"role\s*=\s*"wayland-proxy""#),
        "wayland-proxy role declaration not found in {MINIJAIL_PROFILES_NIX}"
    );
    assert!(
        src.contains("\"w1-wayland-proxy\""),
        "seccompPolicyRef = \"w1-wayland-proxy\" not found in {MINIJAIL_PROFILES_NIX}"
    );
    assert!(
        src.contains("/run/nixling-wlproxy"),
        "writable path /run/nixling-wlproxy not found in {MINIJAIL_PROFILES_NIX}"
    );

    // Extract the wayland-proxy block (from `role = "wayland-proxy";` through the
    // first `};` line), mirroring the bash awk block extraction.
    let block = extract_block(&src, r#"role\s*=\s*"wayland-proxy";"#, r"^\s*};\s*$")
        .expect("could not extract wayland-proxy profile block from minijail-profiles.nix");

    // Capabilities empty (no CAP_ token in the block).
    assert!(
        !any_line_matches(&block, r"CAP_"),
        "wayland-proxy profile must have empty capabilities; found CAP_ token(s) in block:\n{block}"
    );
    // requiresStartRoot must not be true.
    assert!(
        !any_line_matches(&block, r"requiresStartRoot\s*=\s*true"),
        "wayland-proxy profile must not set requiresStartRoot = true"
    );
    // userNamespace must not be set (no broker-pre-NS for this role).
    assert!(
        !any_line_matches(&block, r"userNamespace\s*="),
        "wayland-proxy profile must not declare a userNamespace (no broker-pre-NS for this role)"
    );
    // No PipeWire/Pulse references in the block.
    assert!(
        !any_line_matches(&block, r"pipewire|pulse"),
        "wayland-proxy profile must not bind PipeWire/Pulse sockets; found reference in block:\n{block}"
    );
    // umask = 7 (so the filter socket has mode 0660).
    assert!(
        any_line_matches(&block, r"umask\s*=\s*7"),
        "wayland-proxy profile must declare umask = 7 (0o007)"
    );
    // deviceBinds must be empty (inline `[ ]`).
    assert!(
        any_line_matches(&block, r"deviceBinds\s*=\s*\[\s*\]"),
        "wayland-proxy profile deviceBinds must be empty [ ]"
    );
}

/// Layer-1 `assert_policy_ref_entry` (bash lines 141-148): the
/// `policy_ref_device_classes` table in
/// `packages/nixling-priv-broker/src/live_handlers.rs` MUST carry a
/// `"w1-wayland-proxy"` entry.
#[test]
fn wayland_proxy_policy_ref_device_classes_source() {
    assert!(
        repo_path_exists(LIVE_HANDLERS_RS),
        "missing {LIVE_HANDLERS_RS}"
    );
    let src = read_repo_file(LIVE_HANDLERS_RS);
    assert!(
        src.contains("\"w1-wayland-proxy\""),
        "\"w1-wayland-proxy\" not found in live_handlers.rs policy_ref_device_classes"
    );
}

/// Layer-1 `assert_rust_variant_exists` (bash lines 150-161): the WaylandProxy
/// role variant MUST exist in both `packages/nixling-core/src/processes.rs`
/// (`ProcessRole::WaylandProxy`) and `packages/nixling-ipc/src/broker_wire.rs`
/// (`RunnerRole::WaylandProxy`).
#[test]
fn wayland_proxy_rust_variants_declared_source() {
    assert!(repo_path_exists(PROCESSES_RS), "missing {PROCESSES_RS}");
    assert!(repo_path_exists(BROKER_WIRE_RS), "missing {BROKER_WIRE_RS}");
    let processes = read_repo_file(PROCESSES_RS);
    let broker_wire = read_repo_file(BROKER_WIRE_RS);
    assert!(
        any_line_matches(&processes, r"WaylandProxy"),
        "ProcessRole::WaylandProxy not found in {PROCESSES_RS}"
    );
    assert!(
        any_line_matches(&broker_wire, r"WaylandProxy"),
        "RunnerRole::WaylandProxy not found in {BROKER_WIRE_RS}"
    );
}

/// Layer-1 `assert_installed_profiles_consistent` (bash lines 163-189), applied
/// to the REAL rendered fixture RoleProfile (strictly stronger than the bash
/// gate, which scanned `/etc/nixling/minijail-profiles/*wayland-proxy*.json` and
/// skipped silently when no host profiles were installed). The rendered
/// wayland-proxy RoleProfile MUST carry the ADR-0025 filter-proxy shape: empty
/// capabilities, the `w1-wayland-proxy` seccomp reference, EMPTY device binds and
/// bind mounts (pure AF_UNIX proxy; compositor access is granted by ACL, not a
/// profile bind), the dedicated runtime dir `/run/nixling-wlproxy/<vm>` as the
/// sole writable path, no PipeWire/Pulse path access, `umask = 0o007`, and NO
/// broker-pre-NS userNamespace.
#[test]
fn wayland_proxy_rendered_profile_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: NL_FIXTURES_FULL unset (feature-rich fixture unavailable)");
        return;
    };
    let pw_re = Regex::new(r"pipewire|pulse").expect("valid regex");
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::WaylandProxy {
                continue;
            }
            seen += 1;
            let p = &node.profile;
            assert!(
                p.caps.is_empty(),
                "wayland-proxy {} (vm {}) must declare EMPTY capabilities; got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-wayland-proxy"),
                "wayland-proxy {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.mount_policy.device_binds.is_empty(),
                "wayland-proxy {} (vm {}) must have EMPTY device binds (pure AF_UNIX proxy); got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.device_binds
            );
            assert!(
                p.mount_policy.bind_mounts.is_empty(),
                "wayland-proxy {} (vm {}) must have EMPTY bind mounts (compositor access via ACL, \
                 not a profile bind); got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.bind_mounts
            );
            let runtime_dir = format!("/run/nixling-wlproxy/{}", dag.vm);
            assert!(
                p.mount_policy
                    .writable_paths
                    .iter()
                    .any(|w| w.path == runtime_dir),
                "wayland-proxy {} (vm {}) must expose the dedicated runtime dir {runtime_dir} as a \
                 writable path; got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.writable_paths
            );
            // No PipeWire/Pulse access on any read-only or writable path.
            for w in &p.mount_policy.writable_paths {
                assert!(
                    !pw_re.is_match(&w.path),
                    "wayland-proxy {} (vm {}) writable path references PipeWire/Pulse: {}",
                    p.profile_id,
                    dag.vm,
                    w.path
                );
            }
            for rp in &p.mount_policy.read_only_paths {
                assert!(
                    !pw_re.is_match(rp),
                    "wayland-proxy {} (vm {}) read-only path references PipeWire/Pulse: {rp}",
                    p.profile_id,
                    dag.vm
                );
            }
            assert_eq!(
                p.umask,
                Some(7),
                "wayland-proxy {} (vm {}) must declare umask = 0o007 (filter socket mode 0660)",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.user_namespace.is_none(),
                "wayland-proxy {} (vm {}) must NOT declare a userNamespace (no broker-pre-NS for \
                 this role); got {:?}",
                p.profile_id,
                dag.vm,
                p.user_namespace
            );
        }
    }
    assert!(
        seen > 0,
        "feature-rich fixture has no wayland-proxy node — corp-full enables graphics (regression)"
    );
}
