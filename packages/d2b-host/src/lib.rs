//! `d2b-host` is the disjoint host-prepare API surface. The modules
//! below are file-disjoint contract boundaries:
//!
//! | Area  | Owns                                                                 |
//! | ----- | -------------------------------------------------------------------- |
//! | cg    | [`cgroup`]                                                            |
//! | net   | [`ifname`], [`netlink`], [`routes`], [`bridge_port`]                  |
//! | nft   | [`nftables`]                                                          |
//! | host  | [`modules`], [`devices`], [`ioctl_policy`]                            |
//! | tests | `fake` backends gated behind the `fake-backends` feature              |
//!
//! Crate-level invariants:
//!
//! - `#![forbid(unsafe_code)]`: any required `unsafe` (e.g. raw netlink
//!   FFI, SCM_RIGHTS fd handling) lives in `d2b-broker`'s
//!   quarantined `sys.rs`, never here.
//! - No dependency on `d2bd` or `d2b-broker`. This crate
//!   is consumed by both; the dependency direction is one-way.

#![forbid(unsafe_code)]

pub mod bridge_port;
pub mod cgroup;
pub mod devices;
pub mod ifname;
pub mod ioctl_policy;
pub mod modules;
// BPF seccomp compilation from the ioctl_policy matrix.
// Lives here (not d2b-core) so DeviceClass is available without
// a dep-graph cycle; d2b-broker converts CompiledSeccompProgram
// to libc::sock_filter in its quarantined sys.rs.
pub mod netlink;
pub mod nftables;
pub mod routes;
pub mod seccomp;
// Neutral Volume effect-port composition wrapper. Concrete broker-backed
// implementations are supplied by the Zone runtime.
pub mod volume_effect_adapter;
// Hardlink-farm primitive for per-VM store activation. Same-filesystem
// check + per-generation marker + atomic current-symlink swap with crash
// reconciliation.
pub mod hardlink_farm;
pub mod host_generation;
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

// v1.1.1 RenderDnsmasqEnvConf daemon-host-prep DAG op support.
// Per ADR 0018. Pure-Rust dnsmasq config
// rendering from typed env metadata; the broker writes the
// rendered file to /var/lib/d2b/dnsmasq/<env>.conf
// atomically (broker-only sole-writer invariant). Replaces the
// retired host-singleton dnsmasq render path.
pub mod dnsmasq;
