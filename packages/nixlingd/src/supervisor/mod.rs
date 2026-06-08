//! Per-VM supervisor surface, owned by W3 scope **s1** (pidfd
//! integration). Scopes that follow extend this module with their own
//! supervisor surface.

pub mod pidfd;
pub mod pidfd_table;
// W4-H4: pure per-VM DAG executor over nixling_core::processes::VmProcessDag.
// Trait-based NodeRunner abstraction so the orchestration logic is
// testable without a real broker; the production daemon wires the
// W4-H5 SpawnRunner broker variant behind the trait.
pub mod dag;
// W4-H6: daemon state persistence + restart reconciliation. Pure
// proc/<pid>/stat field-22 parser + (pid, start_time_ticks)
// classification surface; production FilesystemSnapshotStore writes
// to /var/lib/nixling/daemon-state/<vm>/runtime.<role>.json.
pub mod state;
// P2 ph2-p2-stop-dag-owner: typed stop-DAG planner that reconciles
// nftables fragments and USBIP carriers on daemon restart / vm_stop
// against the bundle's declared intent. Pure planner — dispatch
// happens via the existing ApplyNftables / UsbipBind / UsbipUnbind
// broker ops.
pub mod stop_dag;
