# USBIP per-busid state machine

> Reference for the typed, fail-fast state machine that the d2b
> daemon (via the privileged broker) drives every time a USBIP
> passthrough device is attached to a target VM.
>
> Source: [`packages/d2bd/src/usbip_state_machine.rs`](../../packages/d2bd/src/usbip_state_machine.rs).
> Canonical-order anchor: [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care).

## Why a state machine

The host-side USBIP path is a chain of cooperating subsystems —
the `usbip-host` kernel module, a per-busid file lock under
`/run/d2b/locks/usbip/<busid>`, the per-env nftables carve-out
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

## Prerequisites

The executor may run only after `d2bd` has resolved the trusted bundle for
the target VM/env/busid. Required preconditions are:

- a USBIP bind intent and firewall intent exist for the VM/busid;
- VM apply paths that need guest import have a running VM and authenticated
  guest-control USBIP capability;
- `d2b.site.yubikey.enable = true` and at least one enabled VM in the env
  opts into `usbip.yubikey = true` before host YubiKey machinery is expected;
- the broker can prepare `usbip-host`, the host-session busid lock,
  backend/export carrier, and per-env proxy; and
- physical topology/policy checks allow the observed device before exposure.

Public remediation stays on the lifecycle surface: start the VM with
`d2b vm start <vm> --apply`, reconcile with
`d2b usb attach <vm> <busid> --apply`, or release with
`d2b usb detach <vm> <busid> --apply`. Operators must not edit lock files
or sysfs driver links directly.

## Canonical order

The bring-up order is:

```text
modprobe → lock → withhold → firewall → backend → bind → proxy
```

`backend` and `proxy` are per-env sidecar readiness checks. They are
not per-busid resources, and the current proxy is a generic L4 TCP
forwarder (`socat TCP-LISTEN ... TCP:127.0.0.1:<backendPort>`), not a
USBIP protocol or busid-aware process.

Single-busid teardown therefore reverses only per-busid mutable state:

```text
bind → firewall → withhold → lock → modprobe
```

`modprobe` at the tail of the stop path is intentionally a no-op
— the kernel module stays loaded. The per-env backend/proxy sidecars
remain running during a single-VM restart or detach so active same-env
streams are not bounced. Before the `bind` stop step writes the
usbip-host driver unbind control, the broker asks the per-device
`usbip_sockfd` control to shut down any socket-backed stream and treats
only already-gone/already-disconnected socket races as benign. The
implementation does not claim a generic sysfs revoke: missing driver
unbind support, a stuck helper, or ACL-revoke failure is surfaced with
manual recovery guidance while the session busid claim remains in place.

| Step | Step kind | Backing broker op / daemon action | Why this position |
| --- | --- | --- | --- |
| 1 | `modprobe` | `ModprobeIfAllowed { module: "usbip-host" }` against the trusted-bundle kernel-module matrix | Every later step silently no-ops without the kernel symbol surface. |
| 2 | `lock` | broker-written owner record at `/run/d2b/locks/usbip/<busid>` for the target VM, read by the daemon for status/reconcile | Single owner per busid, regardless of env. |
| 3 | `withhold` | daemon-side admission gate that refuses non-owner-env `SpawnRunner` requests for the same busid | Closes the race window before the firewall opens. |
| 4 | `firewall` | `UsbipBindFirewallRule { bundle_usbip_firewall_intent_ref }` | Per-env `inet d2b` carve-out so the per-env proxy can accept the bind. |
| 5 | `backend` | `SpawnRunner { role: RunnerRole::Usbip, vm_id: sys-<env>-usbipd, … }` | Ensure the per-env usbipd backend runner is up. Idempotent; not stopped for one busid. |
| 6 | `bind` | `UsbipBind { bus_id, vm }` | Kernel binds the physical device to the per-env usbipd backend. |
| 7 | `proxy` | generic per-env TCP proxy listen socket open | Target VM can now attach to the bound device. Idempotent; not busid-aware and not stopped for one busid. |

## Typed surface

The state machine is fully typed; rearranging or skipping steps
is a compile-time error.

```rust
use d2bd::usbip_state_machine::{
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
* `UsbipExecutionReport::failure_rollback_order()` — returns only
  successful per-busid steps in reverse order for failure rollback,
  filtering out shared per-env backend/proxy sidecar checks.

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
| `message` | `usbip busid '<busid>' refused at step '<step>': <reason>` |
| `remediation` | Names the busid and gives a concise probe/fix/retry recovery step. |

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
  error. The stop-path / reconciler uses
  `report.failure_rollback_order()` rather than a raw reverse of
  `report.completed`, preserving per-env backend/proxy sidecars.

The executor MUST treat each step as idempotent so retries after
a partial failure are safe.

## Per-env proxy synchronization

The daemon encodes the current generic L4 proxy strategy in
`UsbipProxySynchronizationPlan`
([`packages/d2bd/src/usbip_reconcile_state.rs`](../../packages/d2bd/src/usbip_reconcile_state.rs)).
The encoded strategy deliberately avoids busid-aware claims that the current
`socat` proxy cannot satisfy:

* **Attach / single-VM restart:** optimistically refresh backend/export
  readiness and verify the per-env proxy listener. Do not stop, rebind, or
  recycle the proxy, so unrelated same-env streams stay up.
* **Single-busid release:** before host stream shutdown or `usbip unbind`, prove
  that the firewall carve-out can be blocked/withdrawn and that any established
  stream can be terminated by exact VM/proxy tuple cleanup whose source identity
  is not hidden by SNAT and whose anti-spoofing posture is proven. If the
  reconciler cannot prove that ordering and tuple,
  `usbip-revocation-not-isolated` includes the target VM and busid, fails closed,
  and   preserves the broker-owned session busid lock for manual drain/recovery rather than
  pretending the generic proxy selectively closed that busid.
* **Targeted cleanup (future/explicit):** only after a proven stream tuple or a
  busid-aware proxy implementation exists may the daemon run targeted cleanup.
  The firewall carve-out is withdrawn before any flow kill, so a killed TCP
  stream cannot immediately reconnect. TCP may use exact conntrack deletion
  and/or exact established-socket kill by VM/proxy tuple; UDP may use exact
  conntrack deletion only. SNAT-obscured sources, unproven anti-spoofing, shared
  listeners, and ambiguous same-env streams are never killed for a single busid.
* **Proxy recycle:** bouncing same-env active streams is allowed only through an
  explicit bounded-drain or force policy. Any implementation that rebinds the
  proxy socket must hold an exclusive socket lifecycle lock (or use socket
  activation) and perform fd-relative socket-path handling before the rebind.

## VM lifecycle carrier cleanup

VM stop/restart uses `UsbipVmCarrierCleanupPlan` to detach any guest import and
drain host-side active carrier state only when the selected stream can first be
isolated. The host-session per-busid claim is preserved on VM stop/restart so
the same VM can start again and reattach through the normal bind path during the
current host boot/session. It is not preserved across host reboot because the
lock is under `/run/d2b/locks/usbip`. Only an explicit USB detach may revoke
backend ACLs and release the claim during a host session, and only after
firewall withdrawal/targeted flow cleanup and host unbind succeed. A
dead/unreachable VM guest-detach failure stays visible as degraded cleanup but
does not block host-side firewall withdrawal or unbind.

The cleanup plan never stops or rebinds the per-env backend/proxy sidecars. If a
selected stream cannot be isolated from unrelated same-env traffic, cleanup fails
closed before sysfs `usbip-host` unbind, keeps the session claim, and surfaces
manual recovery instead of killing the shared listener.

VM start treats same-host-session same-VM USBIP session claims as required until an explicit
optional-device policy exists. Runtime absence, guest-control import failure, or
per-env proxy/backend unavailability degrades the USB row and lets boot continue
with a precise remediation command. A same-owner row where the host claim is held,
the device is already bound to `usbip-host`, and the guest import is detached is
convergable: the daemon may refresh the firewall/proxy path and ask guestd to
import the busid again without releasing the host-session claim.

During backend ACL grant the broker treats `/dev/bus/usb/<bus>/<dev>` as a
volatile device node. It may retry across transient devnum changes or brief
sysfs `ENOENT` windows only while the busid, VID/PID, bus number, and physical
port-chain identity remain stable. ACLs granted to any previously observed
device node are revoked before retry/failure; missing old nodes are benign
because the kernel removes them during re-enumeration. VID/PID or topology
changes still fail closed.
without exposing the device. Required policy failures — missing or mismatched
vendor/product allowlists, undeclared physical topology, or topology mismatch —
fail before device exposure and roll back the VM start with remediation to fix
the declaration or bind the approved physical device.

`d2b usb probe` and `d2b status` project this split directly: session
claim, host bind/carrier/proxy, guest import, topology/policy, degraded
reasons, and remediation commands are separate fields. A same-VM session claim
that has not reconverged its active carriers is degraded, not `bound`.

## Recovery pointers

This page is the state-machine reference. Operator procedures live in
[Troubleshoot USBIP passthrough](../how-to/troubleshoot-usbip.md), which maps
probe/status symptoms to lifecycle commands without asking operators to mutate
locks, sysfs driver links, nftables rules, or per-env sidecars directly.

## Tests

| Layer | Path | What it asserts |
| --- | --- | --- |
| Unit | `packages/d2bd/src/usbip_state_machine.rs` (`mod tests`) | `CANONICAL_STEPS` is pinned, `stop_order()` and failure rollback preserve per-env backend/proxy sidecars, every step's failure surfaces as `TypedError::UsbipStepFailed`, and the typed-error envelope carries exit code 67. |
| Unit | `packages/d2bd/src/usbip_reconcile_state.rs` (`mod tests`) | VM stop/restart carrier cleanup preserves session claims, explicit detach releases only after successful cleanup, failures preserve claims/manual recovery, firewall-before-flow-kill ordering holds, and same-env sidecars are not bounced. |
| Contract | [`packages/d2b-contract-tests/tests/policy_supervisor.rs`](../../packages/d2b-contract-tests/tests/policy_supervisor.rs) (`usbip_state_machine_surface`) | Module is wired into `lib.rs`; canonical order is pinned in source; typed-error variant + exit code 67 are wired; this doc names the canonical order verbatim. |

## See also

* [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care)
  — the binding canonical-order statement.
* [`docs/reference/privileges.md`](./privileges.md) §`Usbip` —
  per-env runner / broker op surface that backs each step.
* [`docs/reference/components-usbip.md`](./components-usbip.md)
  — operator-facing USBIP component reference.
* [`tests/unit/nix/cases/usbip-gating.nix`](../../tests/unit/nix/cases/usbip-gating.nix)
  — eval-time gate that host-side USBIP artifacts require both
  host and per-VM opt-ins.
