//! Per-VM supervisor surface for pidfd integration and follow-on
//! supervisor modules.

pub mod pidfd;
pub mod pidfd_table;
// Observe-only runner-liveness probe consulted by the readiness wait
// loop so a runner that dies before its readiness socket appears
// fast-fails instead of blocking to the readiness deadline.
pub mod readiness_liveness;
// Pure per-VM DAG executor over nixling_core::processes::VmProcessDag.
// Trait-based NodeRunner abstraction so the orchestration logic is
// testable without a real broker; the production daemon wires the
// SpawnRunner broker variant behind the trait.
pub mod dag;
// Daemon state persistence + restart reconciliation. Pure proc/<pid>/stat
// field-22 parser + (pid, start_time_ticks)
// classification surface; production FilesystemSnapshotStore writes
// to /var/lib/nixling/daemon-state/<vm>/runtime.<role>.json.
pub mod state;
// Typed stop-DAG planner that reconciles nftables fragments and USBIP
// carriers on daemon restart / vm_stop
// against the bundle's declared intent. Pure planner — dispatch
// happens via the existing ApplyNftables / UsbipBind / UsbipUnbind
// broker ops.
pub mod stop_dag;
