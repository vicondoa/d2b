# USBIP per-busid state machine

> Reference for the provider-owned, typed USBIP planning and characterization
> model. It documents the canonical per-busid ordering and provider-local
> tests; it is not a production daemon call path.
>
> Source: [`packages/d2b-provider-device-usbip/src/state_machine.rs`](../../packages/d2b-provider-device-usbip/src/state_machine.rs).
> The production-wired lifecycle path is the Provider's
> [`lifecycle.rs`](../../packages/d2b-provider-device-usbip/src/lifecycle.rs) - the
> USB Service and USB Binding lifecycles behind the family's typed driver seam
> ([`driver.rs`](../../packages/d2b-provider-device-usbip/src/driver.rs)), which the
> daemon's production port runs with the zone-wide authority ledger
> ([`packages/d2bd/src/usbip_production.rs`](../../packages/d2bd/src/usbip_production.rs)).
> [`reconcile_state.rs`](../../packages/d2b-provider-device-usbip/src/reconcile_state.rs)
> retains only the Provider-owned degraded-reason vocabulary that the daemon
> projects into the probe and status surfaces.
> Canonical-order anchor: [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care).

## Why a state machine

The host-side USBIP path is a chain of cooperating subsystems -
the `usbip-host` kernel module, a per-busid file lock under
`/run/d2b/locks/usbip/<busid>`, the per-env nftables carve-out
(`UsbipBindFirewallRule`), the per-env usbipd backend + proxy
runners, and the per-busid `UsbipBind { bus_id, vm }` operation
itself. Any step out of order silently corrupts state:

* Binding before `modprobe usbip-host` succeeds returns a
  confusing `ENODEV` deep inside the broker call site.
* Skipping the per-busid lock lets two envs race for the same
  physical device - both win briefly, then one loses on the first
  I/O.
* Opening the firewall before withholding non-owner-env
  `SpawnRunner`s leaves a brief window where another env's
  backend can accept the connection.
* Starting the proxy before the backend is up means the first
  guest USB transfer races readiness and looks like a
  `usbip: error: connect failed`.

The state machine pins the order so call sites can't shuffle it.

## Model scope

The planning model is pure provider code and has no production consumer in the
current tree. A future effect adapter must establish these preconditions before
using it for a live operation:

- a USBIP bind intent and firewall intent exist for the VM/busid;
- VM apply paths that need guest import have a running VM and an authenticated
  ComponentSession USBIP Process;
- `d2b.site.yubikey.enable = true` and at least one enabled VM in the env
  opts into `usbip.yubikey = true` before host YubiKey machinery is expected;
- the broker can prepare `usbip-host`, the host-session busid lock,
  backend/export carrier, and per-env proxy; and
- physical topology/policy checks allow the observed device before exposure.

Public remediation stays on the lifecycle surface: start the VM with
`d2b guest start <name> --apply`, reconcile with
`d2b device usb attach <name> <busid> --apply`, or release with
`d2b device usb detach <name> <busid> --apply`. Operators must not edit lock files
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

`modprobe` at the tail of the stop path is intentionally a no-op -
the kernel module stays loaded. The per-env backend/proxy sidecars
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
use d2b_provider_device_usbip::{
    build_usbip_plan, execute_usbip_plan,
    UsbipBusidPlan, UsbipBusidStep, UsbipPlanError, UsbipStepExecutor,
};
```

* [`UsbipBusidStep`] - enum, one variant per canonical step.
* [`UsbipBusidPlan`] - `{ busid, env, vm, steps }`. `steps` is
  pinned to `CANONICAL_STEPS` at construction.
* [`build_usbip_plan(busid, env, vm, resolver)`] - pure
  constructor. Consults the `BundleResolver` so the per-env
  firewall intent (`usbip-fw:env:<env>:bus:<busid>`) and the
  per-(env, vm, busid) bind intent
  (`usbip-bind:env:<env>:vm:<vm>:bus:<busid>`) are both proven
  to exist in the trusted bundle BEFORE the executor ever runs.
* [`UsbipStepExecutor`] - trait, one method per step. Provider-local tests
  inject a fixture executor that records call order and can fail a chosen
  step; no production adapter currently implements this trait.
* [`execute_usbip_plan(plan, executor)`] - drives the plan
  top-to-bottom, fail-fast on the first error.
* `UsbipExecutionReport::failure_rollback_order()` - returns only
  successful per-busid steps in reverse order for failure rollback,
  filtering out shared per-env backend/proxy sidecar checks.

## Failure mode

Any step's failure is returned to the adapter as:

```rust
UsbipPlanError {
    busid: String,
    step: UsbipBusidStep,
    reason: String,
}
```

The provider error is an internal typed result for this planning model. No
current daemon or broker adapter maps it into a public error envelope.

### Partial-progress contract

`execute_usbip_plan` returns either:

* `Ok(UsbipExecutionReport)` - `report.completed` is the full
  `CANONICAL_STEPS` list; `report.failed` is `None`.
* `Err((UsbipExecutionReport, UsbipPlanError))` - `report.completed`
  holds every step that succeeded before the failure;
  `report.failed = Some((step, reason))` matches the typed
  error. The stop-path / reconciler uses
  `report.failure_rollback_order()` rather than a raw reverse of
  `report.completed`, preserving per-env backend/proxy sidecars.

The executor MUST treat each step as idempotent so retries after
a partial failure are safe.

## Proxy ownership and release order

The v3 reconcile model that encoded the shared-listener proxy strategy
(`UsbipProxySynchronizationPlan`) went with the rest of the unreachable model.
The live path
([`lifecycle.rs`](../../packages/d2b-provider-device-usbip/src/lifecycle.rs))
makes the Binding the release unit: it owns its Guest attachment, its private
proxy, and one Service slot, while the Service owns Host-global authority, the
physical bus binding, and the multiplexed relay
([`workers.rs`](../../packages/d2b-provider-device-usbip/src/workers.rs)).

* **Attach / single-VM restart:** a Binding activates only once its Service is
  ready; it acquires the Service slot, then its private proxy, then the Guest
  attach Process. It never stops, rebinds, or recycles the shared backend or
  the per-Network relay, so unrelated Bindings' streams stay up.
* **Single-busid release:** a Binding finalizes its own Guest Endpoint, attach
  Process, private proxy, and Service slot, and it has no access to Service
  authority or unbind - so it cannot release a physical device that another
  Binding still uses. The Service does not unbind its owned device until every
  Binding has closed: the supervisor drains them first, then releases relay and
  physical authority. Restart adopts only a matching child identity, and an
  ambiguous identity is quarantined without a destructive effect.
* **Revocation that cannot be isolated:** the retained provider vocabulary
  ([`reconcile_state.rs`](../../packages/d2b-provider-device-usbip/src/reconcile_state.rs))
  reports the closed degraded reasons for this path (`ProxyUnavailable`,
  `HostBindUnavailable`, `StaleHostState`, `StaleGuestState`,
  `LockHeldByOtherOwner`), and its remediation stays fail-closed on the
  broker-owned session claim: cleanup refused before `usbip-host` unbind is
  retried only once a single targeted stream can be proven, or the VM is
  stopped so USBIP streams drain. The planning model's stop path records the
  same contract - selective revocation relies on host unbind plus targeted
  conntrack/socket cleanup, or fails closed when the selected stream cannot be
  isolated.
* **Proxy recycle:** bouncing same-Network active streams is allowed only
  through an explicit bounded-drain or force policy. Any implementation that
  rebinds a proxy socket must hold an exclusive socket lifecycle lock (or use
  socket activation) and perform fd-relative socket-path handling before the
  rebind.

## VM lifecycle carrier cleanup

VM stop/restart detaches the Binding's Guest attachment and drains the carrier
state the Binding owns; the Service-owned physical bind and the broker-owned
session claim stay in place. The deleted reconcile model's
`UsbipVmCarrierCleanupPlan` no longer exists - the Binding finalizer in
[`lifecycle.rs`](../../packages/d2b-provider-device-usbip/src/lifecycle.rs)
carries the cleanup, and the daemon's probe/status projection reports what is
left behind. The host-session per-busid claim is preserved on VM stop/restart so
the same VM can start again and reattach through the normal bind path during the
current host boot/session. It is not preserved across host reboot because the
lock is under `/run/d2b/locks/usbip`. Only an explicit USB detach may revoke
backend ACLs and release the claim during a host session, and only after
firewall withdrawal/targeted flow cleanup and host unbind succeed. A
dead/unreachable VM target-local detach failure stays visible as degraded
cleanup, and the Service keeps the owned bind - with the broker keeping the
session claim for manual recovery - until the Binding closes or an operator
clears it.

A single-binding release never stops or rebinds the shared backend or the
per-Network relay. If a selected stream cannot be isolated from unrelated
same-Network traffic, cleanup fails closed before sysfs `usbip-host` unbind,
keeps the session claim, and surfaces manual recovery instead of killing the
shared listener.

VM start treats same-host-session same-VM USBIP session claims as required until an explicit
optional-device policy exists. Runtime absence, target-local import failure, or
per-env proxy/backend unavailability degrades the USB row and lets boot continue
with a precise remediation command. A same-owner row where the host claim is held,
the device is already bound to `usbip-host`, and the guest import is detached is
convergable: the daemon may refresh the firewall/proxy path and ask the
target-local USBIP Process to import the busid again without releasing the
host-session claim.

During backend ACL grant the broker treats `/dev/bus/usb/<bus>/<dev>` as a
volatile device node. It may retry across transient devnum changes or brief
sysfs `ENOENT` windows only while the busid, VID/PID, bus number, and physical
port-chain identity remain stable. ACLs granted to any previously observed
device node are revoked before retry/failure; missing old nodes are benign
because the kernel removes them during re-enumeration. VID/PID or topology
changes still fail closed.
without exposing the device. Required policy failures - missing or mismatched
vendor/product allowlists, undeclared physical topology, or topology mismatch -
fail before device exposure and roll back the VM start with remediation to fix
the declaration or bind the approved physical device.

`d2b device usb probe` and `d2b guest status <name>` project this split directly: session
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
| Unit | `packages/d2b-provider-device-usbip/src/state_machine.rs` (`mod tests`) | `CANONICAL_STEPS` is pinned, `stop_order()` and failure rollback preserve per-env backend/proxy sidecars, and step failures remain typed provider results. |
| Integration | `packages/d2b-provider-device-usbip/tests/service_binding_lifecycle.rs` | Binding activation acquires the Service slot, then the private proxy, then the Guest Process; wrong-zone/opt-out and authority conflicts refuse before any bind; a matching restart identity adopts while a stale one quarantines; a Binding closes its Process before the Service unbinds, and one Binding finalizes without unbinding the shared Service. |
| Integration | `packages/d2b-provider-device-usbip/tests/production_port.rs` | The production port keeps Binding teardown ahead of Service release. |

## See also

* [AGENTS.md "Critical subsystems"](../../AGENTS.md#critical-subsystems--handle-with-care) -
  the binding canonical-order statement.
* [`docs/reference/privileges.md`](./privileges.md) §`Usbip` -
  per-env runner / broker op surface that backs each step.
* [`docs/reference/components-usbip.md`](./components-usbip.md) -
  operator-facing USBIP component reference.
* [`packages/d2b-provider-device-usbip/nix/tests/default.nix`](../../packages/d2b-provider-device-usbip/nix/tests/default.nix) -
  owner-local Nix evaluation for the USBIP guest module.
