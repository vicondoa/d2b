# U36 d2b-provider-display-wayland

net: -384 lines, -0 deps

- delete: `src/wayland_proxy/attribution.rs` whole module (169 lines + `pub mod attribution;` in mod.rs) — zero callers in crate or workspace; only its own tests exercise `ClientAttributionBook`/`ProxyClientId`/`ClientAttribution`. labs/window-chrome/proxy carries its own copy (refused #S27 covers that copy, not this dead crate copy). [packages/d2b-provider-display-wayland/src/wayland_proxy/attribution.rs] (leaf)
- delete: process.rs test-only grant/ticket surface (~147 lines) — `LaunchGrants::issue_for_supervisor` (14), `from_supervisor` 2-arg (15), `from_supervisor_for_session` (18), `into_parts` (14), `into_worker_tickets` (14), `into_worker_tickets_with_fence` (25), `LaunchTicket::new` (17), `new_with_generation` (18), `compositor_grant`/`gpu_grant` accessors (12) — all `#[allow(dead_code)]`, only test callers (process.rs tests); production path uses `issue_for_supervisor_with_controller_generation` (d2bd interaction_composition.rs:4381) + `into_worker_tickets_with_fence_and_controller` (controller.rs:1050). [packages/d2b-provider-display-wayland/src/process.rs] (leaf)
- delete: controller.rs `principal_release_receipt` (9 lines) — `#[allow(dead_code)]`, only test caller (controller.rs:1520); production releases via `release_session_principal`. [packages/d2b-provider-display-wayland/src/controller.rs] (leaf)
- delete: clipboard.rs `object_forwarding` + `ClipboardObjectForwarding` (31 lines) — zero callers anywhere (only its own tests); filter.rs uses only `global_disposition`/`ClipboardMimePolicy`/`ClipboardRoute`/`MimeDecision`. [packages/d2b-provider-display-wayland/src/wayland_proxy/clipboard.rs] (leaf)
- delete: wayland_proxy/policy.rs `GlobalOverride` (8 lines) — zero callers anywhere in workspace (not even its own tests); `PolicyInput` carries the same fields directly. [packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs] (leaf)
- delete: bridge.rs test-only surface (~19 lines) — `BridgeConfig::disabled()` (6), `impl Default for BridgeReconnectPolicy` (8), `recv_flags_are_fail_closed` (2), `SCM_RIGHTS_MIN_FDS`/`SCM_RIGHTS_MIN_CONTROL_BYTES`/`SCM_RIGHTS_CONTROL_FD_SLOTS` (3) — read only by filter.rs tests (3072-3086) and bridge.rs tests; bin/filter production use `from_identity_parts` + explicit reconnect policy. [packages/d2b-provider-display-wayland/src/wayland_proxy/bridge.rs] (leaf)

## Checked

Read all src files (lib, controller 1673, process 1312, runtime 1164, policy 424, spec 445, session_children 421, principal 121, wayland_proxy/* incl. filter 3441, decoration 2334, dmabuf 1239, bridge 643, policy 977, diag, clipboard, identity, readiness, attribution, bin 1068), tests/, BUILD.bazel, Cargo.toml, nix/, README. Caller verification method: workspace-wide `grep -rn` over packages/ + nixos-modules/ + labs/ + docs/reference for every flagged symbol (attribution symbols, object_forwarding, GlobalOverride, each process.rs constructor/accessor, disabled/default/recv_flags/SCM_RIGHTS, principal_release_receipt), plus bin/filter production-section scans (filter tests start at 2883) and d2bd/wayland-policy/wayland-session cross-crate import census (`d2b_provider_display_wayland::` in dependents: SERVICE_PACKAGE, GraceState, DisplayIdentity, FinalizationReport, ReconcileResult, Phase, LaunchTicket, AttachmentGrantHandle, ProcessObservation, session_children::display_owned_child_intents + wayland_session_resource_projection — all live). No zero-caller claim left unverified; every surviving pub item has at least one production caller.

## Honored rows (U1 ledger, verbatim; no re-flag)

- #S27 [refused] labs/window-chrome/proxy copy — stays refused: standalone workspace, ADR 0047 owns disposition; labs has its own copies of attribution/bridge/etc., does not import this crate's wayland_proxy. Not re-flagged.
- #S28 [applied] legacy border-decoration renderer + wayland_proxy_argv.rs, ui-colors.nix, niri-vm-borders.nix, metrics.rs, audit.rs, portal.rs, descriptor.rs, bundle half of PrincipalPool, stale ProxyProcessTemplate — verified absent at HEAD (ls + grep: no such files, no ProxyProcessTemplate). Not re-flagged.
- #S29 [applied] src/wayland_proxy_argv.rs — absent at HEAD. Not re-flagged.
- #S37 [partial] hand-written Wayland global catalog, FilterInput::debug_logging, lib.rs re-export block — global catalog (KNOWN_GLOBALS) still live (policy.rs:8, tests assert unknown-interface rejection), debug_logging still a pinned wire field (policy.rs:48, provider_behavior.rs round-trip test), lib.rs re-export block consumed by d2bd/wayland-policy/wayland-session. Refused parts stay refused; no new evidence. Not re-flagged.

## Reopened refusals

None. No ledger row's blocking condition changed at HEAD (3f2664794).