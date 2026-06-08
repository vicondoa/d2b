# USBIP per-busid state machine

> Reference for the typed, fail-fast state machine that the nixling
> daemon (via the privileged broker) drives every time a USBIP
> passthrough device is attached to a target VM.
>
> Source: [`packages/nixlingd/src/usbip_state_machine.rs`](../../packages/nixlingd/src/usbip_state_machine.rs).
> Plan row: USBIP state-machine hardening.
> Canonical-order anchor: [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care).

## Why a state machine

The host-side USBIP path is a chain of cooperating subsystems —
the `usbip-host` kernel module, a per-busid file lock under
`/run/nixling/locks/usbip/<busid>`, the per-env nftables carve-out
(`UsbipBindFirewallRule`), the per-env usbipd backend + proxy
runners, and the per-busid `UsbipBind { bus_id, vm }` operation
itself. Any step out of order silently corrupts state:

* Binding before `modprobe usbip-host` succeeds returns a
  confusing `ENODEV` deep inside the broker call site.
* Skipping the per-busid lock lets two envs race for the same
  physical device — both win briefly, then one loses on the first
  I/O.
* Opening the firewall before withholding non-owner-env
  `SpawnRunner`s leaves a brief window where another env's
  backend can accept the connection.
* Starting the proxy before the backend is up means the first
  guest USB transfer races readiness and looks like a
  `usbip: error: connect failed`.

The state machine pins the order so call sites can't shuffle it.

## Canonical order

The bring-up order is:

```text
modprobe → lock → withhold → firewall → backend → bind → proxy
```

The stop path is the same list reversed:

```text
proxy → bind → backend → firewall → withhold → lock → modprobe
```

`modprobe` at the tail of the stop path is intentionally a no-op
— the kernel module stays loaded — but the executor's stop-side
dispatch table stays aligned with the bring-up table.

| Step | Step kind | Backing broker op / daemon action | Why this position |
| --- | --- | --- | --- |
| 1 | `modprobe` | `ModprobeIfAllowed { module: "usbip-host" }` against the trusted-bundle kernel-module matrix | Every later step silently no-ops without the kernel symbol surface. |
| 2 | `lock` | daemon-side `flock` on `/run/nixling/locks/usbip/<busid>` for the target env | Single owner per busid, regardless of env. |
| 3 | `withhold` | daemon-side admission gate that refuses non-owner-env `SpawnRunner` requests for the same busid | Closes the race window before the firewall opens. |
| 4 | `firewall` | `UsbipBindFirewallRule { bundle_usbip_firewall_intent_ref }` | Per-env `inet nixling` carve-out so the per-env proxy can accept the bind. |
| 5 | `backend` | `SpawnRunner { role: RunnerRole::Usbip, vm_id: sys-<env>-usbipd, … }` | Per-env usbipd backend runner. Idempotent. |
| 6 | `bind` | `UsbipBind { bus_id, vm }` | Kernel binds the physical device to the per-env usbipd backend. |
| 7 | `proxy` | per-env usbipd proxy listen socket open | Target VM can now attach to the bound device. |

## Typed surface

The state machine is fully typed; rearranging or skipping steps
is a compile-time error.

```rust
use nixlingd::usbip_state_machine::{
    build_usbip_plan, execute_usbip_plan,
    UsbipBusidPlan, UsbipBusidStep, UsbipStepExecutor,
};
```

* [`UsbipBusidStep`] — enum, one variant per canonical step.
* [`UsbipBusidPlan`] — `{ busid, env, vm, steps }`. `steps` is
  pinned to `CANONICAL_STEPS` at construction.
* [`build_usbip_plan(busid, env, vm, resolver)`] — pure
  constructor. Consults the `BundleResolver` so the per-env
  firewall intent (`usbip-fw:env:<env>:bus:<busid>`) and the
  per-(env, vm, busid) bind intent
  (`usbip-bind:env:<env>:vm:<vm>:bus:<busid>`) are both proven
  to exist in the trusted bundle BEFORE the executor ever runs.
* [`UsbipStepExecutor`] — trait, one method per step. Production
  wires this through the broker dispatch surface; tests inject a
  fixture executor that records call order and can fail a chosen
  step.
* [`execute_usbip_plan(plan, executor)`] — drives the plan
  top-to-bottom, fail-fast on the first error.

## Failure mode

Any step's failure is normalised to:

```rust
TypedError::UsbipStepFailed {
    busid: String,
    step: UsbipBusidStep,
    reason: String,
}
```

with these envelope fields:

| Field | Value |
| --- | --- |
| `kind` | `usbip-step-failed` |
| `exit_code` | `67` |
| `message` | `usbip per-busid state machine refused at step '<step>' for busid '<busid>': <reason>` |
| `remediation` | Names the canonical order verbatim, plus the per-step recovery hint. |

Exit code 67 is distinct from the adjacent surfaces:

| Code | Surface |
| --- | --- |
| 64 | `host-kernel-modules-missing` (broader matrix) |
| 65 | `otel-host-bridge-readiness-timeout` |
| 66 | `net-route-preflight-degraded` |
| **67** | **`usbip-step-failed` (per-busid state machine)** |

so operators can grep for it across hosts independently of the
broader kernel-module check, observability bridge, or
net-route-degraded paths.

### Partial-progress contract

`execute_usbip_plan` returns either:

* `Ok(UsbipExecutionReport)` — `report.completed` is the full
  `CANONICAL_STEPS` list; `report.failed` is `None`.
* `Err((UsbipExecutionReport, TypedError))` — `report.completed`
  holds every step that succeeded before the failure;
  `report.failed = Some((step, reason))` matches the typed
  error. The stop-path / reconciler uses `report.completed` (in
  reverse) to undo the partial bring-up.

The executor MUST treat each step as idempotent so retries after
a partial failure are safe.

## Operator remediation

| Failing step | First thing to check |
| --- | --- |
| `modprobe` | `usbip-host` is in the trusted-bundle kernel-module matrix and `ModprobeIfAllowed` is permitted. |
| `lock` | Another env already owns `/run/nixling/locks/usbip/<busid>` — stop the owner first, then retry. |
| `withhold` | A non-owner-env `SpawnRunner` for the same busid is in flight; let it drain or stop the offending env's VM. |
| `firewall` | Re-render the trusted bundle so `UsbipBindFirewallRule` exists for this env/busid. |
| `backend` | The per-env usbipd backend `SpawnRunner` failed — inspect the broker audit log (`/var/lib/nixling/audit/broker-<utc-date>.jsonl`). |
| `bind` | The kernel `UsbipBind` op refused — confirm the bundle's `vendor_product_allowlist` matches the physical device. |
| `proxy` | The per-env usbipd proxy listen socket failed to open; almost always a stale backend from a previous run that the stop path didn't unwind. |

## Tests

| Layer | Path | What it asserts |
| --- | --- | --- |
| Unit | `packages/nixlingd/src/usbip_state_machine.rs` (`mod tests`) | `CANONICAL_STEPS` is pinned, `stop_order()` reverses it, every step's failure surfaces as `TypedError::UsbipStepFailed`, and the typed-error envelope carries exit code 67. |
| Integration (eval) | [`tests/usbip-state-machine-eval.sh`](../../tests/usbip-state-machine-eval.sh) | Module is wired into `lib.rs`; canonical order is pinned in source; typed-error variant + exit code 67 are wired; this doc names the canonical order verbatim. |

## See also

* [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care)
  — the binding canonical-order statement.
* [`docs/reference/privileges.md`](./privileges.md) §`Usbip` —
  per-env runner / broker op surface that backs each step.
* [`docs/reference/components-usbip.md`](./components-usbip.md)
  — operator-facing USBIP component reference.
* [`tests/usbip-gating-eval.sh`](../../tests/usbip-gating-eval.sh)
  — eval-time gate that the host-side USBIP units are only
  emitted when both host and per-VM opt-ins are set.
