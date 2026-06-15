# Stop-DAG reconcile owner

> Status: typed planner; live host probes and broker dispatch wiring
> land alongside the broader
> daemon-only cut.

## Why

When `nixlingd` starts (cold boot, supervisor crash-recovery, or after
operator restart) it cannot assume the host's nftables fragments and
USBIP busid carriers still match the trusted bundle:

- The bundle may have been rebuilt while the daemon was down, so the
  per-table ownership-hash recorded under
  `/var/lib/nixling/state/host-runtime.json` will no longer match the
  current bundle's desired hash.
- A previously-managed VM may have died (or been removed from the
  bundle) leaving a stale `iif <bridge> ... usbip` rule fragment and
  a bound USBIP carrier nobody owns.
- A declared, autostart-eligible VM may have been listed as the owner
  of a busid that the host has not yet bound.

The stop-DAG owner is the typed planner that turns "declared
intent (from `BundleResolver`) versus actual host state" into a
typed [`ReconcileReport`](#reconcilereport) the supervisor dispatches
through the **existing** broker ops. No new wire variants are
introduced; all converge actions map 1:1 to `ApplyNftables`,
`UsbipBind`, or `UsbipUnbind`.

## Surface

Module: `packages/nixlingd/src/supervisor/stop_dag.rs`.

```text
pub struct StopDagOwner;
pub struct ObservedHostState { … }
pub struct ReconcileReport   { nftables_actions, usbip_actions }

impl StopDagOwner {
    pub fn reconcile_on_restart(resolver: &BundleResolver) -> ReconcileReport;
    pub fn reconcile(resolver: &BundleResolver,
                     observed:  &ObservedHostState) -> ReconcileReport;
}
```

### `ObservedHostState`

Snapshot of the host as it currently exists. Production callers
populate it from the daemon's restart-reconciliation probes
(`/var/lib/nixling/state/host-runtime.json` + `/run/nixling/locks/usbip/`);
unit tests populate it directly to simulate drift.

| Field | Source on a real host |
| --- | --- |
| `nft_applied_hashes: BTreeMap<intent_id, String>` | last `desired_hash` written by the broker after `ApplyNftables` for that intent. Missing entry → never applied. |
| `usbip_bound_busids: BTreeSet<String>`            | busids currently exported via `usbip-host` on the host. |
| `active_vms: BTreeSet<String>`                    | VMs the supervisor is currently tracking via `pidfd_table`. |

### Drift classification

`NftablesDriftReason`:

| Variant | Meaning | Broker op |
| --- | --- | --- |
| `NeverApplied`                | No prior apply hash recorded. | `ApplyNftables` |
| `HashMismatch { observed, desired }` | Recorded hash ≠ bundle's `desired_hash`. | `ApplyNftables` |

`UsbipDriftReason`:

| Variant | Meaning | Broker op |
| --- | --- | --- |
| `CarrierMissing { vm, env }` | Bundle declares this busid for a VM in `active_vms`; no host carrier present. | `UsbipBind` |
| `OwnerInactive { last_owner }` | Carrier present, but the owning VM is not in `active_vms`. | `UsbipUnbind` |
| `Undeclared`                  | Carrier present for a busid the bundle no longer mentions. | `UsbipUnbind` |

### `ReconcileReport`

Deterministic, sorted by intent id / busid. `is_noop()` returns true
when both action vectors are empty — the supervisor uses that to
skip the broker dispatch on a clean restart.

## When the supervisor calls it

1. **Daemon startup**, before the daemon serves the first mutating
   verb. The startup path:
   - reconnects to live runner pidfds under
     `/run/nixling/state/runner-pidfds/`,
   - calls `StopDagOwner::reconcile_on_restart(&resolver)`,
   - dispatches each action through the existing
     `ApplyNftables` / `UsbipBind` / `UsbipUnbind` broker requests,
   - persists the new `host-runtime.json` hash on success.
2. **`vm_stop`**, after the per-VM drain → SIGTERM → broker
   `ApplyNftables` (remove per-VM carve-out) sequence completes.
   The same planner is invoked with the just-stopped VM removed
   from `active_vms`, so any orphaned USBIP carrier surfaces as
   `OwnerInactive` and is unbound on the same critical path.

## Testing

- Unit tests (`packages/nixlingd/src/supervisor/stop_dag.rs::tests`)
  exercise every drift variant against a fixture-derived
  `BundleResolver`.
- `packages/nixling-contract-tests/tests/policy_daemon.rs`
  (`stop_dag_reconcile_surface`) is a static gate that asserts
  the module surface, the supervisor module wires it in, the planner
  composes only existing broker ops (no new `*Request` types), and
  this doc is up to date.

## See also

- [`docs/reference/host-prep-dag.md`](./host-prep-dag.md) —
  startup-side DAG (mirror image of the stop-DAG).
- [`docs/reference/privileges.md`](./privileges.md) — broker op
  catalogue (`ApplyNftables`, `UsbipBind`, `UsbipUnbind`).
- [`docs/explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
  — daemon restart contract this planner feeds into.
