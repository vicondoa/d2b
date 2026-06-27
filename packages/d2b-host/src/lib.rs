//! `d2b-host` is the disjoint host-prepare API surface. The modules
//! below are file-disjoint contract boundaries:
//!
//! | Area  | Owns                                                                 |
//! | ----- | -------------------------------------------------------------------- |
//! | cg    | [`cgroup`]                                                            |
//! | net   | [`ifname`], [`netlink`], [`routes`], [`bridge_port`]                  |
//! | nft   | [`nftables`]                                                          |
//! | host  | [`modules`], [`devices`], [`ioctl_policy`]                            |
//! | tests | [`fake`] backends gated behind the `fake-backends` feature            |
//!
//! Crate-level invariants:
//!
//! - `#![forbid(unsafe_code)]`: any required `unsafe` (e.g. raw netlink
//!   FFI, SCM_RIGHTS fd handling) lives in `d2b-priv-broker`'s
//!   quarantined `sys.rs`, never here.
//! - No dependency on `d2bd` or `d2b-priv-broker`. This crate
//!   is consumed by both; the dependency direction is one-way.

#![forbid(unsafe_code)]

pub mod bridge_port;
pub mod cgroup;
pub mod devices;
pub mod fake;
pub mod ifname;
pub mod ioctl_policy;
pub mod modules;
// BPF seccomp compilation from the ioctl_policy matrix.
// Lives here (not d2b-core) so DeviceClass is available without
// a dep-graph cycle; d2b-priv-broker converts CompiledSeccompProgram
// to libc::sock_filter in its quarantined sys.rs.
pub mod netlink;
pub mod nftables;
pub mod routes;
pub mod seccomp;
// Runner-shape preflight + CH net-handoff probe.
pub mod runner_shape;
// Static runner lifecycle metadata used by host-side argv dispatch.
pub mod runner_process;
// Pure CH argv generator. Consumed by d2bd via the SpawnRunner
// broker wire.
pub mod ch_argv;
// Pure QEMU media argv scaffold. It emits a paused QMP-ready baseline for
// broker-owned media fd passing without exposing media paths.
pub mod qemu_media_argv;
// Pure virtiofsd argv generator (one instance per `microvm.shares` row;
// consumed by d2bd via SpawnRunner).
pub mod virtiofsd_argv;
// Pure swtpm argv generator (long-lived `swtpm socket ...` plus the
// pre-start `swtpm_ioctl -i --unix ...` flush per the
// VmProcessInvariants::swtpm_pre_start_flush invariant).
pub mod swtpm_argv;
// Pure crosvm device gpu sidecar argv generator (one per graphics-enabled
// VM; consumed by d2bd via SpawnRunner with RunnerRole::Gpu).
pub mod gpu_argv;
// Pure vhost-device-sound audio sidecar argv generator.
pub mod audio_argv;
// Pure crosvm device video-decoder sidecar argv generator.
pub mod video_argv;
// Pure socat-based vsock-relay argv generator (covers the guest-egress +
// stack-vm-listen shapes documented in
// nixos-modules/components/observability/{host,guest,stack}.nix).
pub mod vsock_relay_argv;
// Pure `usbip bind|unbind --busid <bus-id>` argv generator. The generator
// stands alone with a bus-id shape validator.
pub mod usbip_argv;
// Pure OTel host-bridge argv generator. Replaces the singleton
// d2b-otel-host-bridge.service with a broker SpawnRunner under
// RunnerRole::OtelHostBridge.
pub mod otel_host_bridge_argv;
// Host-jailed Wayland filter proxy argv generator. Emits argv for the
// per-VM d2b-<vm>-wlproxy role spawned via RunnerRole::WaylandProxy.
pub mod wayland_proxy_argv;
// Hardlink-farm primitive for per-VM store activation. Same-filesystem
// check + per-generation marker + atomic current-symlink swap with crash
// reconciliation.
pub mod hardlink_farm;
// Live ssh-keygen fingerprint + public-key probe wrapping ssh-keygen -lf
// and ssh-keygen -y -f for the broker-side rotate / trust / show ops.
pub mod ssh_keygen;
// Typed declaration + pure enforcer for the per-VM state-directory
// ownership matrix under /var/lib/d2b/vms/<vm>/. CRITICAL: includes
// the hardlink-farm carve-out so recursive ownership ops never leak into
// /nix/store.
pub mod ownership_matrix;
// Typed host-prep DAG executed by the daemon on every VM start. Replaces
// the per-VM `microvm-tap-interfaces@` + `microvm-setup@` systemd
// templates.
pub mod host_prep_dag;
// Pure qemu-media physical USB identity/preflight helpers. Live sysfs reads,
// registry writes, udev reloads, and fd opens stay in the privileged broker.
pub mod media;

// Canonical Rust-side runner argv regenerator.
// Documents the migration surface from the Nix-side argv
// generation in processes-json.nix to the typed Rust generators
// in this crate (ch_argv, virtiofsd_argv, gpu_argv, audio_argv,
// swtpm_argv, usbip_argv, video_argv, vsock_relay_argv,
// otel_host_bridge_argv). See ADR 0018.
pub mod runner_argv_regenerator;
// Host-side runtime provider adapters. The concrete Cloud Hypervisor
// adapter wraps typed CH argv input without serializing argv/paths into
// provider DTOs.
pub mod runtime_provider;

// v1.1.1 RenderDnsmasqEnvConf daemon-host-prep DAG op support.
// Per ADR 0018. Pure-Rust dnsmasq config
// rendering from typed env metadata; the broker writes the
// rendered file to /var/lib/d2b/dnsmasq/<env>.conf
// atomically (broker-only sole-writer invariant). Replaces the
// retired host-singleton dnsmasq render path.
pub mod dnsmasq;
