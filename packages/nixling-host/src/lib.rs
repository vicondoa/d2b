//! `nixling-host` is the disjoint host-prepare API surface landed by
//! the W3 integrator API/contract prep commit. The module stubs below
//! are the file-disjoint contract boundaries that the parallel scope
//! agents s1-s4 will fill in:
//!
//! | Scope | Owns                                                                 |
//! | ----- | -------------------------------------------------------------------- |
//! | s1    | [`cgroup`]                                                            |
//! | s2    | [`ifname`], [`netlink`], [`routes`], [`bridge_port`]                  |
//! | s3    | [`nftables`]                                                          |
//! | s4    | [`modules`], [`devices`], [`ioctl_policy`]                            |
//! | tests | [`fake`] backends gated behind the `fake-backends` feature            |
//!
//! Crate-level invariants:
//!
//! - `#![forbid(unsafe_code)]`: any required `unsafe` (e.g. raw netlink
//!   FFI, SCM_RIGHTS fd handling) lives in `nixling-priv-broker`'s
//!   quarantined `sys.rs`, never here.
//! - No dependency on `nixlingd` or `nixling-priv-broker`. This crate
//!   is consumed by both; the dependency direction is one-way.

#![forbid(unsafe_code)]

pub mod bridge_port;
pub mod cgroup;
pub mod devices;
pub mod fake;
pub mod ifname;
pub mod ioctl_policy;
pub mod modules;
pub mod netlink;
pub mod nftables;
pub mod routes;
// W3 s4 begin: scope-owned runner-shape preflight + CH net-handoff probe.
pub mod runner_shape;
// W3 s4 end.
// W4-H1: pure CH argv generator. Consumed by nixlingd via the
// W4-H5 SpawnRunner broker wire.
pub mod ch_argv;
// W4-H2: pure virtiofsd argv generator (one instance per
// `microvm.shares` row; consumed by nixlingd via SpawnRunner).
pub mod virtiofsd_argv;
// W4-H3: pure swtpm argv generator (long-lived `swtpm socket ...`
// plus the pre-start `swtpm_ioctl -i --unix ...` flush per the W3
// VmProcessInvariants::swtpm_pre_start_flush invariant).
pub mod swtpm_argv;
// W5-H1: pure crosvm device gpu sidecar argv generator (one per
// graphics-enabled VM; consumed by nixlingd via SpawnRunner with
// RunnerRole::Gpu in W5-fu).
pub mod gpu_argv;
// W5-H2: pure vhost-device-sound audio sidecar argv generator.
pub mod audio_argv;
// W5-H3: pure crosvm device video-decoder sidecar argv generator.
pub mod video_argv;
// W6-H1: pure socat-based vsock-relay argv generator (covers the
// guest-egress + stack-vm-listen shapes documented in
// nixos-modules/components/observability/{host,guest,stack}.nix).
pub mod vsock_relay_argv;
// W6-H2: pure `usbip bind|unbind --busid <bus-id>` argv generator.
// W6-fu wires this into the broker's UsbipBind variant; today the
// generator stands alone with a bus-id shape validator.
pub mod usbip_argv;
// P1 observability-4 + decision 5: pure OTel host-bridge argv
// generator. Replaces the singleton nixling-otel-host-bridge.service
// with a broker SpawnRunner under RunnerRole::OtelHostBridge.
pub mod otel_host_bridge_argv;
// W7-fu: hardlink-farm primitive for per-VM store activation.
// Same-filesystem check + per-generation marker + atomic
// current-symlink swap with crash reconciliation.
pub mod hardlink_farm;
// W8-fu: live ssh-keygen fingerprint + public-key probe wrapping
// ssh-keygen -lf and ssh-keygen -y -f for the broker-side rotate /
// trust / show ops.
pub mod ssh_keygen;
// P2 ph2-p2-ownership-matrix: typed declaration + pure enforcer for
// the per-VM state-directory ownership matrix under
// /var/lib/nixling/vms/<vm>/. CRITICAL: includes the hardlink-farm
// carve-out so recursive ownership ops never leak into /nix/store.
pub mod ownership_matrix;
// P2 ph2-dag-host-prep: typed host-prep DAG executed by the daemon
// on every VM start. Replaces the per-VM `microvm-tap-interfaces@`
// + `microvm-setup@` systemd templates.
pub mod host_prep_dag;
