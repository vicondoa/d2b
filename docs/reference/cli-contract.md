# nixling CLI contract

**Diataxis category:** reference.

This document is the command contract for the single user-facing
`nixling` entry point. It covers the CLI surfaces that are fully
owned in Rust, including the read-only and daemon-backed commands
that go through `nixlingd`.

Examples use the smoke topology from the Layer-1 fixtures: one workload
VM (`corp-vm`) and one auto-declared net VM (`sys-work-net`) in the
`work` environment. Human examples are representative snapshots rather
than literal byte-for-byte goldens unless the corresponding
`tests/golden/cli-output/*` fixture has landed.

## Scope and conventions

> There is no bash fallback. The Rust CLI never executes bash, and
> the no-bash invariant is enforced by
> `tests/no-bash-exec-eval.sh`. Verbs that used to degrade to bash on
> `not-yet-implemented` or `daemon-down` now surface typed
> envelopes (`not-yet-implemented` exit 78, `daemon-down` exit 1) —
> see [`error-codes.md` § "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions)
> for the multi-line block format used on those envelopes. `nixling
> up/down/restart/list` are first-class top-level aliases for `vm
> start/stop/restart/list` and route through the same daemon path.

- Command headings use the dispatched **leaf** form (`keys list`,
  `audio mic`, `status --check-bridges`) because disposition is
  assigned at that granularity.
- `--json` always means: emit one newline-terminated JSON document on
  stdout and keep progress or warnings on stderr.
- Non-zero rows link into [`error-codes.md`](./error-codes.md) only when
  the failure is part of the stable typed-error model. Success and raw
  POSIX signal exits are listed inline without a docs anchor.
- There is no bash CLI fallback. Commands marked `retired-bash`
  return a typed exit-78 envelope when invoked; commands marked
  `rust-native shim` own help / argument parsing in Rust but still
  return the same typed exit-78 envelope where their daemon-native
  backends are not yet shipped; commands marked `rust-native` stay on
  the daemon/public-socket or native planner path.

## Command reference

### `list`

**Synopsis:** `nixling list [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the stable machine-readable inventory document on stdout instead of the human table. |
| `--human` | boolean | `false` | Force the human inventory table on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | The inventory query is always global; it does not accept a VM selector. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Unexpected daemon reply, local probe, or manifest-read failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling list
NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
corp-vm            work      false     false false   10.20.0.10      running
sys-work-net       work      false     false false   192.0.2.1       running (net-vm)
```

**`--json` example** — schema: [`list.schema.json`](./cli-output/list.schema.json); prose companion: [`list.md`](./cli-output/list.md).

```json
[
  {
    "name": "corp-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "10.20.0.10",
    "status": "running",
    "isNetVm": false
  },
  {
    "name": "sys-work-net",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "192.0.2.1",
    "status": "running",
    "isNetVm": true
  }
]
```

**Status**

`list` is a daemon-native, read-only inventory query. When nixlingd is
reachable, the CLI queries the public socket and reports declared VM
metadata plus daemon-derived lifecycle state. If the public socket is
unavailable or does not support the request, the CLI falls back to the
static manifest/local status path.

**Native**

- Pure read-only public-socket inventory query; no broker op and no guest
  contact. The static manifest/local status path is fallback only.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

Rows are ordered by VM name because the historical bash implementation iterated `jq keys[]`; the current daemon-native path keeps that ordering contract.

### `realm enter`

**Synopsis:** `nixling realm enter <realm>`

Enters the local gateway VM for a gateway-backed realm by opening an
interactive `vm exec` session to the gateway workload. The host resolves
the realm through the generated realm entrypoint table and verifies the
gateway VM is declared and running before attempting the exec. Realm
relay/provider credentials remain inside the gateway guest.

### `realm run`

**Synopsis:** `nixling realm run <realm> -- <argv...> [--human | --json]`

Runs a one-shot command inside the local gateway VM for a
gateway-backed realm. This is the low-level escape hatch for scripts that
need to issue an exact command in the realm trust boundary. Daily
workload operations should continue to use `nixling vm ...`; realm
targets route through the configured gateway entrypoint when supported.

### Realm target routing

`nixling vm start|stop|restart|exec <workload>.<node>.<realm>.nixling`
keeps local VM names on the existing host fast path. Fully qualified
realm targets are resolved through the generated `realm-entrypoints.json`
table. Missing entrypoints fail closed with an actionable
`missing-realm-entrypoint` error; stopped gateway VMs fail with
`gateway-not-running` and a remediation command to start the gateway.

`nixling vm list --realm <realm>` runs `nixling vm list` inside the
realm gateway through the same local guest-control exec path. It does not
make the host persist a remote node/workload registry.

### `vm display`

**Synopsis:**

- `nixling vm display list [--target <nl://...>] [--human | --json]`
- `nixling vm display close <session-id> [--human | --json]`

`vm display` manages active gateway display sessions. It requires the
gateway daemon's public socket and does not fall back to SSH or host-side
Wayland setup. `list` returns only bounded non-secret session metadata:
session id, realm target, lifecycle state, authorizing operation id, and
principal. `close` asks the gateway daemon to tear down the listener plus the
provider-side display agent when the session is still active, and reports
`closed = false` for an already-absent session.

`--json` for `list` emits:

```json
{
  "command": "vm display list",
  "target": "nl://demo.gw.work.nixling",
  "sessions": [
    {
      "sessionId": "s0",
      "target": "nl://demo.gw.work.nixling",
      "state": "running",
      "operationId": "gw-exec-1",
      "principal": "uid-1000"
    }
  ]
}
```

The response never contains app argv, socket paths, relay endpoints,
credentials, file descriptors, pidfds, cgroup paths, namespace ids, or
process output. See
[display and virtual I/O capabilities](./display-io-capabilities.md) for the
capability boundary.

### `vm start`

**Synopsis:** `nixling vm start <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`vm start` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `nixling: NOTICE: defaulting to
--dry-run` and the CLI defaults to `--dry-run`; `--apply` routes
through the daemon.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the 5-node per-VM DAG without spawning any role. |
| `--apply` | boolean | `false` | Perform the lifecycle mutation. |
| `--json` | boolean | `false` | Emit the dry-run DAG or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `nixling up` exit table is preserved in this
file as history.

**Human example**

```text
$ nixling vm start corp-vm --apply
vm start corp-vm: spawned pid=4242 start_time_ticks=123456789
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: `nixlingd` → broker `SpawnRunner`.

**Bash**

- There is no bash execution path for this verb.
### `vm stop`

**Synopsis:** `nixling vm stop <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`vm stop` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `nixling: NOTICE: defaulting to
--dry-run` and the CLI defaults to `--dry-run`; `--apply` routes
through the daemon.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the 5-node per-VM DAG without spawning any role. |
| `--apply` | boolean | `false` | Perform the lifecycle mutation. |
| `--json` | boolean | `false` | Emit the dry-run DAG or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `nixling down` exit table is preserved in this
file as history.

Pidfd `EPERM` while stopping a per-VM-UID runner used to surface as
typed `broker-error` exit 78. Current `--apply` recovers that specific
case by asking the broker to run `SignalRunner`; if the broker reports
`signaled=true`, `vm stop` exits 0. True broker failures — unreachable
broker, dispatch errors, unexpected responses, or `signaled=false` —
still surface as `broker-error` / exit 78.

**Human example**

```text
$ nixling vm stop corp-vm --apply
vm stop corp-vm: broker recorded the audited SignalRunner request for role ch-runner (signal=term, signaled=true)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: `nixlingd` → broker `SignalRunner`.

**Bash**

- There is no bash execution path for this verb.
### `vm restart`

**Synopsis:** `nixling vm restart <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`vm restart` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `nixling: NOTICE: defaulting to
--dry-run` and the CLI defaults to `--dry-run`; `--apply` routes
through the daemon.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the 5-node per-VM DAG without spawning any role. |
| `--apply` | boolean | `false` | Perform the lifecycle mutation. |
| `--json` | boolean | `false` | Emit the dry-run DAG or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `nixling restart` exit table is preserved in
this file as history.

**Human example**

```text
$ nixling vm restart corp-vm --apply
vm restart corp-vm: vm stop corp-vm: broker recorded the audited SignalRunner request for role ch-runner (signal=term, signaled=true); vm start corp-vm: spawned pid=4242 start_time_ticks=123456789
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: `nixlingd` → broker `SignalRunner` for the stop phase,
  then `SpawnRunner` for the start phase.

**Bash**

- There is no bash execution path for this verb.

### `vm list`

**Synopsis:** `nixling vm list [--human] [--json]`

**Status:** `vm list` is the daemon-side runtime inventory surface. It
queries nixlingd's public socket and returns the same live lifecycle/runtime
entries the daemon exposes to desktop clients. If the public socket is not
available, the command exits successfully with an empty `entries` array plus
a note explaining that nixlingd must be started or restarted.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the daemon runtime inventory document on stdout. |
| `--human` | boolean | `false` | Force the human runtime inventory table on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Inventory is global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Unexpected local JSON serialization failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling vm list
VM        STATE    RUNTIME
corp-vm   running  running
```

**`--json` example**

```json
{
  "command": "vm list",
  "entries": [
    {
      "vm": "corp-vm",
      "name": "corp-vm",
      "env": "work",
      "graphics": false,
      "isNetVm": false,
      "lifecycle": {
        "pendingRestart": false,
        "state": "Running"
      },
      "runtime": {
        "detail": "running"
      },
      "services": {
        "gpu": null,
        "microvm": "running",
        "nixling": "active",
        "snd": null,
        "swtpm": null,
        "video": null,
        "virtiofsd": "running"
      },
      "sshUser": "alice",
      "staticIp": "10.20.0.10",
      "tpm": false,
      "usbip": false
    }
  ]
}
```

When nixlingd's public socket is unavailable, `--json` returns:

```json
{
  "command": "vm list",
  "entries": [],
  "notes": "vm list requires nixlingd's public socket; start or restart nixlingd and retry."
}
```

**Current disposition:** `rust-native` — the Rust CLI owns the stable
daemon-side runtime-view contract and reads it from nixlingd's public socket.

### `status`

**Synopsis:** `nixling status [<vm>] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured status document on stdout. |
| `--human` | boolean | `false` | Force the human status view on stdout. |
| `--vm` | string | `null` | Long-form VM selector equivalent to passing `<vm>` positionally. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Optional VM name. When omitted the human command falls back to the global inventory view and appends the bridge-health table. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Unexpected probe failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag, unsupported `--json` shape, or unknown VM. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling status corp-vm
=== corp-vm ===
nixling@corp-vm: inactive
microvm@corp-vm (backend): inactive
virtiofsd: inactive
interactive: stopped
sshd@10.20.0.10: unreachable
pending-restart: no

=== Bridge health ===
BRIDGE               STATE      ADMIN   EXPECTED     RESULT
br-work-lan          DOWN       up      NO-CARRIER   no-carrier (no workloads up)
br-work-up           DOWN       up      NO-CARRIER   no-carrier (net VM stopped)
```

**`--json` example** — schema: [`status.schema.json`](./cli-output/status.schema.json); prose companion: [`status.md`](./cli-output/status.md).

```json
{
  "name": "corp-vm",
  "env": "work",
  "services": {
    "nixling": "inactive",
    "microvm": "inactive",
    "virtiofsd": "inactive",
    "gpu": null,
    "video": null,
    "snd": null,
    "swtpm": null
  },
  "current": null,
  "booted": null,
  "pendingRestart": false,
  "runtime": "unknown",
  "declaredRoles": [
    "host-reconcile",
    "store-virtiofs-preflight"
  ],
  "readiness": []
}
```

**Status**

`status` is a read-only daemon RPC, including the frozen per-VM JSON
shape. A negotiated guest-control state field is reserved for a future
release and is not present in the current frozen shape; it must never
appear as an ad hoc unversioned key.

**Native**

- Read-only daemon query; renders the human view or the frozen `--json` document.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `status --check-bridges`

**Synopsis:** `nixling status --check-bridges`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--check-bridges` | boolean | `false` | Switch `status` into bridge-only mode. This form rejects `--json` and does not accept a VM argument. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Bridge-only mode is always global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Every declared bridge is in the expected healthy state for the current VM topology. | — |
| `2` | Unsupported combination such as `--json`, extra arguments, or an unknown flag. | [`usage`](./error-codes.md#usage) |
| `4` | A bridge is missing, administratively down, or lacks carrier when carrier is required. | [`bridge-unhealthy`](./error-codes.md#bridge-unhealthy) |

**Human example**

```text
$ nixling status --check-bridges
=== Bridge health ===
BRIDGE               STATE      ADMIN   EXPECTED     RESULT
br-work-lan          DOWN       up      NO-CARRIER   no-carrier (no workloads up)
br-work-up           DOWN       up      NO-CARRIER   no-carrier (net VM stopped)
```

**Status**

The bridge-health probe is part of the read-only status surface, even though reconcile remains deferred.

**Native**

- Read-only bridge-health probe; rejects `--json` and a VM selector.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb attach`

**Synopsis:** `nixling usb attach <vm> <busid> [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the daemon → broker USBIP attach plan plus the authenticated guestd import step without mutating host or guest state. |
| `--apply` | boolean | `false` | Ask `nixlingd` to first reconcile any stale guest-side import through guestd, run `UsbipBind` (acquiring the per-busid lock and validating ownership), apply the USBIP firewall carve-out, ensure the per-env USBIP backend/proxy runners are ready, run `UsbipProxyReconcile`, then ask guestd over authenticated guest-control to run the guest-side `usbip attach -r <usbipdHostIp> -b <busid>`. |
| `--json` | boolean | `false` | Emit the dry-run summary as structured JSON. |
| `--human` | boolean | `false` | Force the human dry-run summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |
| `busid` | Required host USB busid in the canonical `B-P[.P...]` form (for example `1-2` or `2-1.4`). |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `nixlingd` is unreachable, or the daemon returned a non-typed USBIP failure. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Missing VM / busid or another usage error. | [`usage`](./error-codes.md#usage) |
| `78` | The daemon reached the broker but the native USBIP apply path was refused. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ nixling usb attach corp-vm 1-2 --dry-run
nixling usb attach --dry-run: would bind and lock, apply the USBIP firewall carve-out, ensure the per-env backend/proxy for busid '1-2' for vm 'corp-vm', reconcile the USBIP proxy, and ask guestd to import the device
```

**Status**

The native CLI sends one intent to `nixlingd`; the daemon drives broker host
USBIP state and authenticated guestd import cleanup/attach over guest-control.

**Native**

- `--apply` routes through `nixlingd` → broker + guestd. There is no SSH fallback for USBIP.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb detach`

**Synopsis:** `nixling usb detach <vm> <busid> [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the daemon → broker USBIP unbind plan without mutating host state. |
| `--apply` | boolean | `false` | Ask `nixlingd` to run `UsbipUnbind` followed by `UsbipProxyReconcile` for the selected VM/busid pair. |
| `--json` | boolean | `false` | Emit the dry-run summary as structured JSON. |
| `--human` | boolean | `false` | Force the human dry-run summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |
| `busid` | Required host USB busid in the canonical `B-P[.P...]` form. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `nixlingd` is unreachable, or the daemon returned a non-typed USBIP failure. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Missing VM / busid or another usage error. | [`usage`](./error-codes.md#usage) |
| `78` | The daemon reached the broker but the native USBIP apply path was refused. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ nixling usb detach corp-vm 1-2 --dry-run
nixling usb detach --dry-run: would unbind busid '1-2' for vm 'corp-vm', and reconcile the USBIP proxy
```

**Status**

The native CLI drives the daemon → broker `UsbipUnbind` / `UsbipProxyReconcile` path directly.

**Native**

- `--apply` routes through `nixlingd` → broker `UsbipUnbind` then `UsbipProxyReconcile`.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb probe`

**Synopsis:** `nixling usb probe [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured USBIP probe inventory instead of the human table. |
| `--human` | boolean | `false` | Force the human USBIP probe table on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | The probe always lists every daemon-declared USBIP busid claim. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `nixlingd` is unreachable or does not expose the native USBIP probe request. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | The daemon reached the broker but the `UsbipProxyReconcile` pass failed. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ nixling usb probe
VM                       ENV          BUSID        STATUS   OWNER
corp-vm                  work         1-2          bound    corp-vm
```

**Status**

Probe is a read-only daemon RPC backed by the broker's `UsbipProxyReconcile` validation pass.

**Native**

- Read-only daemon query enumerating every declared USBIP busid claim.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `console`

**Synopsis:** `nixling console <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | Serial console access has no command-line flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required headless VM name. Graphics VMs are rejected and must be launched with `nixling vm start <vm> --apply`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **v1.0 disposition** — typed `#not-yet-implemented` envelope (the daemon-native console surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015 and the disposition note below). v1.0 invocation returns this exit code unconditionally; the multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the migration guide. v1.2+ (unscheduled) implementation MAY lift this to the codes below. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | (v1.2+ unscheduled) Success. | — |
| `1` | (v1.2+ unscheduled) Console launch failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM, missing argument, or graphics VM selected. | [`usage`](./error-codes.md#usage) |
| `130` | (v1.2+ unscheduled) Console session interrupted with SIGINT. | — |

**Human example**

```text
$ nixling console corp-vm
Connected to corp-vm serial console.
Use ~. to detach.
```

**Status**

The Rust CLI owns help and argument validation, but returns a typed exit-78 `not-yet-implemented` envelope (the daemon-native foreground console handoff is queued for a future release; see ADR 0015 and ADR 0017).

**Native**

- Parses and validates arguments natively, then surfaces the typed `not-yet-implemented` envelope (exit `78`).

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio status`

**Synopsis:** `nixling audio status [<vm>]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | Audio status has no flags in the compatibility contract. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Optional VM name. When omitted, the command prints one block per audio-enabled VM. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **v1.0 disposition** — typed `#not-yet-implemented` envelope (the daemon-native audio status surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015 and the disposition note below). v1.0 invocation returns this exit code unconditionally. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 1 — truly deferred) points operators at the migration runbook. v1.2+ (unscheduled) implementation MAY lift this to the codes below. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | (v1.2+ unscheduled) Success. | — |
| `1` | (v1.2+ unscheduled) Unexpected filesystem or sidecar probe failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling audio status corp-vm
audio:    enabled
mic:      off
speaker:  off
sidecar:  inactive
device:   detached
```

**Status**

The Rust CLI owns help and argument validation, but returns a typed exit-78 `not-yet-implemented` envelope (the daemon-native audio-status surface is queued for a future release; see ADR 0015 and ADR 0017).

**Native**

- Parses and validates arguments natively, then surfaces the typed `not-yet-implemented` envelope (exit `78`).

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio mic`

**Synopsis:** `nixling audio mic on|off <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | The direction and state are positional arguments, not flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `state` | Required literal `on` or `off`. Controls the microphone grant only. |
| `vm` | Required VM name. The VM must declare `audio.enable = true`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **v1.0 disposition** — typed `#not-yet-implemented` envelope (the daemon-native audio mic surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015 and the disposition note below). v1.0 invocation returns this exit code unconditionally. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 1 — truly deferred) points operators at the migration runbook. v1.2+ (unscheduled) implementation MAY lift this to the codes below. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | (v1.2+ unscheduled) Success. | — |
| `1` | (v1.2+ unscheduled) Audio state write, sidecar, or hotplug failure. | [`generic`](./error-codes.md#generic) |
| `2` | Bad state literal, unknown VM, or audio not enabled for the VM. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling audio mic on corp-vm
nixling audio: state -> mic=on, speaker=off

audio:    enabled
mic:      on
speaker:  off
sidecar:  active
device:   will-attach-on-next-up
```

**Status**

The Rust CLI owns help and argument validation, but returns a typed exit-78 `not-yet-implemented` envelope (the daemon-native audio-hotplug surface is queued for a future release; see ADR 0015 and ADR 0017).

**Native**

- Parses and validates arguments natively, then surfaces the typed `not-yet-implemented` envelope (exit `78`).

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio speaker`

**Synopsis:** `nixling audio speaker on|off <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | The direction and state are positional arguments, not flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `state` | Required literal `on` or `off`. Controls the speaker grant only. |
| `vm` | Required VM name. The VM must declare `audio.enable = true`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **v1.0 disposition** — typed `#not-yet-implemented` envelope (the daemon-native audio speaker surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015 and the disposition note below). v1.0 invocation returns this exit code unconditionally. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 1 — truly deferred) points operators at the migration runbook. v1.2+ (unscheduled) implementation MAY lift this to the codes below. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | (v1.2+ unscheduled) Success. | — |
| `1` | (v1.2+ unscheduled) Audio state write, sidecar, or hotplug failure. | [`generic`](./error-codes.md#generic) |
| `2` | Bad state literal, unknown VM, or audio not enabled for the VM. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling audio speaker on corp-vm
nixling audio: state -> mic=off, speaker=on

audio:    enabled
mic:      off
speaker:  on
sidecar:  active
device:   will-attach-on-next-up
```

**Status**

The Rust CLI owns help and argument validation, but returns a typed exit-78 `not-yet-implemented` envelope (the daemon-native audio-speaker surface is queued for a future release; see ADR 0015 and ADR 0017).

**Native**

- Parses and validates arguments natively, then surfaces the typed `not-yet-implemented` envelope (exit `78`).

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio off`

**Synopsis:** `nixling audio off <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | The command revokes both directions; there are no flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **v1.0 disposition** — typed `#not-yet-implemented` envelope (the daemon-native audio off surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015 and the disposition note below). v1.0 invocation returns this exit code unconditionally. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 1 — truly deferred) points operators at the migration runbook. v1.2+ (unscheduled) implementation MAY lift this to the codes below. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | (v1.2+ unscheduled) Success. Calling the command against a VM that never had audio enabled is an idempotent no-op. | — |
| `1` | (v1.2+ unscheduled) Audio state write or sidecar failure. | [`generic`](./error-codes.md#generic) |
| `2` | Missing or unknown VM name. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling audio off corp-vm
nixling audio: state -> mic=off, speaker=off

audio:    enabled
mic:      off
speaker:  off
sidecar:  inactive
device:   detached
```

**Status**

The Rust CLI owns help and argument validation, but returns a typed exit-78 `not-yet-implemented` envelope (the daemon-native audio-off shorthand is queued for a future release; see ADR 0015 and ADR 0017).

**Native**

- Parses and validates arguments natively, then surfaces the typed `not-yet-implemented` envelope (exit `78`).

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `build`

**Synopsis:** `nixling build <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | Build does not take command-line flags in v0.4.0 or v1.0. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Nix evaluation/build failure or missing flake context. | [`generic`](./error-codes.md#generic) |
| `2` | Missing or unknown VM name. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling build corp-vm
nixling: building corp-vm closure...
nixling: corp-vm closure → /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-nixos-system-corp-vm
  GC root: /var/lib/nixling/vms/corp-vm/result
```

**Status**

Build is a native non-destructive planner that renders the eval/build preview without falling back to bash.

**Native**

- Native eval/build planner; renders the closure preview and GC-root path.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.
### `switch`

**Synopsis:** `nixling switch <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`switch` is a daemon-native activation verb. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Perform the activation mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling switch` exit table is preserved in this file as history.

**Human example**

```text
$ nixling switch corp-vm --apply
nixling switch --apply executed via the native daemon → broker path (vm=corp-vm, mode=switch, summary=activated, generationNumber=42)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunActivation` with `ActivationMode::Switch` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `boot`

**Synopsis:** `nixling boot <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`boot` is a daemon-native activation verb. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Perform the activation mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling boot` exit table is preserved in this file as history.

**Human example**

```text
$ nixling boot corp-vm --apply
nixling boot --apply executed via the native daemon → broker path (vm=corp-vm, mode=boot, summary=staged for next boot, generationNumber=42)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunActivation` with `ActivationMode::Boot` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `test`

**Synopsis:** `nixling test <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`test` is a daemon-native activation verb. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Perform the activation mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling test` exit table is preserved in this file as history.

**Human example**

```text
$ nixling test corp-vm --apply
nixling test --apply executed via the native daemon → broker path (vm=corp-vm, mode=test, summary=activated until reboot, generationNumber=42)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunActivation` with `ActivationMode::Test` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `rollback`

**Synopsis:** `nixling rollback <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`rollback` is a daemon-native activation verb. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Perform the activation mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling rollback` exit table is preserved in this file as history.

**Human example**

```text
$ nixling rollback corp-vm --apply
nixling rollback --apply executed via the native daemon → broker path (vm=corp-vm, mode=rollback, summary=rolled back, generationNumber=41)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunActivation` with `ActivationMode::Rollback` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


### `generations`

**Synopsis:** `nixling generations <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | Generation listing has no flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `2` | Missing or unknown VM name. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling generations corp-vm
=== Host-side per-VM store generations (/var/lib/nixling/vms/corp-vm/store-meta/generations) ===
  (none yet — run 'nixling build corp-vm')

=== In-VM nix-profile generations ===
  (corp-vm is not running — start it and try again)
```

**Status**

Generations is a native introspection surface that reports current/booted symlink targets without falling back to bash.

**Native**

- Native introspection of host-side and in-VM nix-profile generations.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.
### `gc`

**Synopsis:** `nixling gc [--dry-run | --apply] [--human | --json]`

**Status**

`gc` is a daemon-native host-store maintenance verb. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan host-side garbage collection without deleting store paths. |
| `--apply` | boolean | `false` | Perform host-side garbage collection. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Garbage collection is global; the current native surface does not take a VM argument. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling gc` exit table is preserved in this file as history.

**Human example**

```text
$ nixling gc --apply
nixling gc --apply executed via the native daemon → broker path (retainedStorePaths=12, keepGenerations=None, summary=pruned nixling-managed store roots)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunGc` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `store verify`

**Synopsis:** `nixling store verify <vm> [--repair] [--human | --json]`

**Status**

`store verify` is a daemon-native, broker-backed live-pool integrity
surface for the ADR 0027 split store-view. The CLI is thin: it never reads
`store-view/live` or `store-view/state` directly.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--repair` | boolean | `false` | Request repair through a forced StoreSync republish, followed by a second verify before reporting success. |
| `--json` | boolean | `false` | Emit the signed store-verify JSON envelope. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Live pool is clean, or repair completed successfully. | — |
| `1` | Daemon is unreachable. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `4` | Drift found or integrity remains unknown. | [`drift`](./error-codes.md#drift) |
| `70` | The named VM is not declared or not visible to the caller. | [`not-found`](./error-codes.md#not-found) |
| `78` | Broker/system failure while verifying. | [`broker-error`](./error-codes.md#broker-error) |

**`--json` example** — schema: [`store-verify.schema.json`](./cli-output/store-verify.schema.json); prose companion: [`store-verify.md`](./cli-output/store-verify.md).

```json
{
  "vm": "corp-vm",
  "status": "ok",
  "checked": 42,
  "drifted": 0,
  "repaired": 0,
  "unknown_reason": null,
  "audit_ref": null,
  "remediation": null
}
```

**Native**

- Routes through `nixlingd` → broker `StoreVerify`.
- Verifies the current marker/manifest and top-level `live/` basenames
  and writes host-only integrity records.
- `--repair` never claims success from the StoreSync attempt alone; it
  returns `repaired` only after the post-repair verification is clean.

**Human example**

```text
$ nixling store verify corp-vm
store verify corp-vm: status=ok checked=42 drifted=0 repaired=0
```

**Bash**

- There is no bash execution path for this verb.

### `trust`

**Synopsis:** `nixling trust <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`trust` is a daemon-native host-key TOFU mutation. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the known-host trust update without mutating the managed entry. |
| `--apply` | boolean | `false` | Perform the TOFU trust mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. The live apply path expects the VM to publish a manifest `staticIp` so the host-key operation can target the guest. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `nixling trust` exit table is preserved in this file as history.

**Human example**

```text
$ nixling trust corp-vm --apply
nixling trust --apply executed via the native daemon → broker path (vm=corp-vm, staticIp=10.20.0.10, knownHostsPath=/var/lib/nixling/known_hosts.nixling, updated=true)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunHostKeyTrust` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `rotate-known-host`

**Synopsis:** `nixling rotate-known-host <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`rotate-known-host` is a daemon-native host-key rotation mutation. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan removal and re-pin of the managed known-host entry without mutating the file. |
| `--apply` | boolean | `false` | Perform the known-host rotation mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. The live apply path expects the VM to publish a manifest `staticIp` so the host-key operation can target the guest. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling rotate-known-host corp-vm --apply
nixling rotate-known-host --apply executed via the native daemon → broker path (vm=corp-vm, staticIp=10.20.0.10, knownHostsPath=/var/lib/nixling/known_hosts.nixling, removed=true)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunRotateKnownHost` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


### `keys list`

**Synopsis:** `nixling keys list [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured managed-key inventory instead of the human table. |
| `--human` | boolean | `false` | Force the human managed-key table on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | The list form is always global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `nixlingd` is unreachable; the typed `#daemon-down` envelope is emitted (the v1.0 daemon-only contract — there is no bash fallback; the v1.0 clean-break per ADR 0015 retired the legacy fallback in v1.0). The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 2 — daemon-down rendering pointer) points operators at the daemon-startup runbook. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unsupported invocation shape inherited from the `keys` subcommand dispatcher. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling keys list
VM                       ENV          FINGERPRINT                                                      MANAGED KEY
corp-vm                  work         SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA              /var/lib/nixling/keys/corp-vm_ed25519
```

**`--json` example**

```json
{
  "command": "keys list",
  "entries": [
    {
      "vm": "corp-vm",
      "env": "work",
      "managedKeyPath": "/var/lib/nixling/keys/corp-vm_ed25519",
      "fingerprint": "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      "knownHostsEntry": "10.42.0.11 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockedExampleKeyForDocsOnly corp-vm"
    }
  ]
}
```

**Status**

Keys list is a native inventory preview that reports the managed-key resolution placeholders without falling back to bash.

**Native**

- Native managed-key inventory query over the daemon public socket.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `keys show`

**Synopsis:** `nixling keys show <vm> [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured managed-key record instead of the public-key line. |
| `--human` | boolean | `false` | Force the raw public-key line on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `nixlingd` is unreachable (typed `#daemon-down` envelope; multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the daemon-startup runbook) — OR the daemon returned the request but the key material was unreadable (typed `#generic` envelope; rare). | [`daemon-down`](./error-codes.md#daemon-down) / [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM, missing VM argument, or unreadable key material reported by daemon as an unknown subject. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling keys show corp-vm
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockedExampleKeyForDocsOnly corp-vm_ed25519.pub
```

**Status**

Keys show is a native preview that reports daemon-resolved key metadata placeholders without falling back to bash.

**Native**

- Native per-VM managed-key lookup over the daemon public socket.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.
### `keys rotate`

**Synopsis:** `nixling keys rotate <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`keys rotate` is a daemon-native managed-key mutation. If neither mutation flag is set, the CLI prints the v0.4 parity notice and defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the key rotation without changing managed host keys or guest auth state. |
| `--apply` | boolean | `false` | Perform the managed-key rotation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling keys rotate corp-vm --apply
nixling keys rotate --apply executed via the native daemon → broker path (vm=corp-vm, fingerprint=SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, keyPath=/var/lib/nixling/keys/corp-vm_ed25519)
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `RunKeysRotate` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


### `audit`

**Synopsis:** `nixling audit [--strict] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--strict` | boolean | `false` | Preserve the strict-audit semantics (extra invariants; warns become errors). v1.0+ never falls back to bash regardless of flag; the historical bash-strict path was retired in v1.0 per ADR 0015. |
| `--human` | boolean | `false` when stdout is not a TTY; otherwise effectively `true` unless `--json` is present | Force the human summary format. |
| `--json` | boolean | `false` | Force the JSON document on stdout even on a TTY. The JSON document shape is stable across v1.0 → v1.1 unless a schema bump is annotated in the audit schema (`./cli-output/audit.schema.json`); v1.0 baseline preserves the audit object shape. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Audit is always global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `78` | **`--strict` flag arm only** — `nixling audit --strict` emits typed `#not-yet-implemented` envelope unconditionally regardless of daemon state per [ADR 0017](../adr/0017-no-bash-fallbacks-invariant.md) § "Migration target table" line 91 (the strict-audit surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017) implementation). The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the migration runbook. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | Success (non-`--strict` arm only — `--strict` returns exit 78 unconditionally per above). | — |
| `1` | (Non-`--strict` arm only) `nixlingd` is unreachable; typed `#daemon-down` envelope emitted. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the daemon-startup runbook. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unknown flag or unexpected positional argument. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling audit --human

=== nixling security audit ===

  kvm_dev_mode:                            660 ✓
  wayland_user_in_kvm:                     false ✓

  store_delivery:
    corp-vm: virtiofs
    sys-work-net: erofs
```

**`--json` example** — schema: [`audit.schema.json`](./cli-output/audit.schema.json); prose companion: [`audit.md`](./cli-output/audit.md).

```json
{
  "kvm_dev_mode": "660",
  "wayland_user_in_kvm": false,
  "store_delivery": {
    "corp-vm": "virtiofs",
    "sys-work-net": "erofs"
  },
  "virtiofsd": {},
  "ssh": {
    "host": {
      "PasswordAuthentication": false
    },
    "corp-vm": {
      "PasswordAuthentication": false
    }
  },
  "bridge_isolation": {},
  "autoUpgrade_commits_lock": false,
  "ch_version": "52.0",
  "crosvm_rev": "deadbeefdead",
  "seccomp_rev": "feedfacefeed",
  "ch_crosvm_pair_ok": true,
  "fail2ban_active": true,
  "sidecars_per_vm": {},
  "usbipd_per_env_isolation": {}
}
```

**Status**

Audit is part of the read-only daemon surface and keeps both human and JSON output contracts. `--strict` surfaces a typed `not-yet-implemented` envelope (exit `78`) pending its daemon-native implementation.

**Native**

- Read-only daemon query; `--json` emits the stable audit document.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `host check`

**Synopsis:** `nixling host check [--strict] [--read-only] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--strict` | boolean | `false` | Promote advisory runner-parity and prerequisite warnings to the failure exit code. |
| `--read-only` | boolean | `true` | Compatibility alias that makes the no-mutation posture explicit. The command is always read-only, so the flag is accepted but does not widen capability. |
| `--human` | boolean | `false` | Force the human host-check summary on stdout. |
| `--json` | boolean | `false` | Emit the stable host-check report document on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Host check is always global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | All required checks passed. | — |
| `1` | At least one advisory warning was reported. | [`host-check-warning`](./error-codes.md#host-check-warning) |
| `2` | At least one required check failed. | [`host-check-failure`](./error-codes.md#host-check-failure) |
| `3` | Unknown flag or other usage error. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling host check
PASS
- kernel-version: running kernel 6.8.0 satisfies >= 6.6
- cgroup-v2: /sys/fs/cgroup/cgroup.controllers is present

WARN
- firewalld-coexistence: firewalld is active; coexistence is reported but host rules are not mutated
```

**`--json` example** — schema: [`host-check.schema.json`](./cli-output/host-check.schema.json); prose companion: [`host-check.md`](./cli-output/host-check.md).

```json
{
  "summary": {
    "pass": 3,
    "warn": 1,
    "fail": 0
  },
  "checks": [
    {
      "id": "kernel-version",
      "severity": "pass",
      "required": true,
      "message": "Kernel 6.8.0 satisfies >= 6.6",
      "remediation": null
    },
    {
      "id": "firewalld-coexistence",
      "severity": "warn",
      "required": false,
      "message": "firewalld is active; keep the host ruleset unchanged",
      "remediation": "Use host prepare for automated firewall reconcile."
    }
  ],
  "runnerParity": [
    {
      "vm": "corp-vm",
      "declaredRunner": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-microvm-cloud-hypervisor-corp-vm",
      "runnerParityPath": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-microvm-cloud-hypervisor-corp-vm",
      "runnerParityOk": true
    }
  ]
}
```

**Status**

Host check is a read-only daemon RPC by design; mutation is explicitly handled by host prepare.

**Native**

- Read-only host-posture inventory; never mutates nftables, cgroups, users, or runtime directories.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

The command never mutates nftables, cgroups, users, or runtime directories. `--read-only` is therefore part of the compatibility surface, not a capability toggle.
### `host prepare`

**Synopsis:** `nixling host prepare [--dry-run | --apply] [--human | --json]`

**Status**

`host prepare` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "nixling: NOTICE: defaulting to --dry-run; nixling 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`. `--dry-run` is wired live; `--apply` is **not yet wired** — the daemon-side typed-intent dispatch and bundle resolver that back it are still pending, so it returns the typed `daemon-down` envelope (exit 1) today (use `--dry-run` for now). On a Tier-0 legacy/mixed host, `--apply` is refused with `tier-0-legacy-uses-nixos-module` / `single-writer-conflict` (exit 78).

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the host reconcile without mutating host state. |
| `--apply` | boolean | `false` | Perform the host-reconcile mutation. **Not yet wired** — returns `daemon-down` (exit 1) today; use `--dry-run` for now. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Host reconcile is global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run summary rendered. (Once `--apply` is wired, a successful apply will also exit `0`.) | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Tier-0 all-legacy refusal, Tier-0 mixed single-writer conflict, or typed `broker-error` / `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`tier-0-legacy-uses-nixos-module`](./error-codes.md#tier-0-legacy-uses-nixos-module), [`single-writer-conflict`](./error-codes.md#single-writer-conflict), [`broker-error`](./error-codes.md#broker-error) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling host prepare --dry-run
host prepare --dry-run: would reconcile nftables + routes + sysctls + /etc/hosts + NetworkManager unmanaged state
```

**Native**

- `--apply`: **not yet wired** — returns the typed `daemon-down` envelope (exit 1) today; re-run with `--dry-run` for now. When the daemon-side dispatch ships, `--apply` will route through `nixlingd` → broker; daemon-unreachable will surface `daemon-down` exit 1, native-handler-deferred `not-yet-implemented` exit 78, and `broker-error` exit 78. On a Tier-0 legacy/mixed host `--apply` is refused today (exit 78). The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative (forthcoming): once the public-socket dispatch ships, `--apply` wires `nixlingd` → broker `ApplyNftables` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNmUnmanaged` (broker ops staged in commit `ee6ed0b`; public-socket dispatch pending).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `host destroy`

**Synopsis:** `nixling host destroy [--dry-run | --apply] [--human | --json]`

**Status**

`host destroy` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "nixling: NOTICE: defaulting to --dry-run; nixling 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`. `--dry-run` is wired live; `--apply` is **not yet wired** — the daemon-side typed-intent dispatch and bundle resolver that back it are still pending, so it returns the typed `daemon-down` envelope (exit 1) today (use `--dry-run` for now). On a Tier-0 legacy host, `--apply` is refused with `tier-0-legacy-uses-nixos-module` (exit 78).

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan removal of nixling-owned host reconcile state without mutating host state. |
| `--apply` | boolean | `false` | Perform the host-reconcile teardown. **Not yet wired** — returns `daemon-down` (exit 1) today; use `--dry-run` for now. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Host teardown is global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run summary rendered. (Once `--apply` is wired, a successful apply will also exit `0`.) | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Tier-0 all-legacy refusal or typed `broker-error` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`tier-0-legacy-uses-nixos-module`](./error-codes.md#tier-0-legacy-uses-nixos-module), [`broker-error`](./error-codes.md#broker-error) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling host destroy --dry-run
host destroy --dry-run: no nixling-owned resources to remove
```

**Native**

- `--apply`: **not yet wired** — returns the typed `daemon-down` envelope (exit 1) today; re-run with `--dry-run` for now. When the daemon-side dispatch ships, `--apply` will route through `nixlingd` → broker; daemon-unreachable will surface `daemon-down` exit 1, native-handler-deferred `not-yet-implemented` exit 78, and `broker-error` exit 78. On a Tier-0 legacy host `--apply` is refused today (exit 78). The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative (forthcoming): once the public-socket dispatch ships, `--apply` wires the same broker-op set in reverse order: `ApplyNmUnmanaged` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNftables` (broker ops staged in commit `ee6ed0b`; reverse-order hardening in `b73e28f`; public-socket dispatch pending).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.

### `host migrate-storage`

**Synopsis:** `nixling host migrate-storage [--dry-run | --apply | --rollback --from-checkpoint <id>] [--human | --json]`

**Status**

Plans the one-time breaking storage layout cutover. The current build is
read-only for this verb: `--dry-run` emits a checkpoint ID, the exact
rollback command, preflight requirements, preserved persistent data,
cutover-only cleanup candidates, and fail-closed hazards. `--apply` and
`--rollback` fail closed until the broker-backed mover ships.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | required unless `--apply` or `--rollback` is set | Plan the storage cutover without moving or deleting host state. |
| `--apply` | boolean | `false` | Apply the storage cutover. Currently returns a typed not-implemented envelope. |
| `--rollback` | boolean | `false` | Roll back from a checkpoint. Currently returns a typed not-implemented envelope. |
| `--from-checkpoint` | string | required with `--rollback` | Checkpoint ID from the dry-run plan. |
| `--json` | boolean | `false` | Emit the dry-run plan or typed refusal envelope as JSON. |
| `--human` | boolean | `false` | Emit the human dry-run plan. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Storage cutover planning is global. |

**Dry-run JSON shape**

```json
{
  "command": "host migrate-storage",
  "mode": "dry-run",
  "checkpointId": "storage-cutover-…",
  "rollbackCommand": "nixling host migrate-storage --rollback --from-checkpoint storage-cutover-…",
  "vmCount": 2,
  "vms": ["corp-vm", "work-vm"],
  "preflightRequirements": [
    "all nixling VMs stopped",
    "nixlingd.service stopped",
    "nixling-priv-broker.service stopped",
    "net VMs stopped; guest routing, TAP connectivity, and dependent bridge traffic will be interrupted"
  ],
  "preserve": [
    "per-VM swtpm NVRAM and swtpm identity markers",
    "declared host bridges, TAP naming intent, nftables/NM/networkd ownership metadata, and network-preflight evidence"
  ],
  "cutoverOnlyCleanup": [
    "/run/nixling-gpu",
    "boot-scoped runtime socket files only after all nixling services are stopped"
  ],
  "failClosedHazards": [
    "symlink or path traversal inside any moved path",
    "recursive operations traversing hardlink farms or mutating shared /nix/store inodes",
    "any attempt to unlink lock files during cutover rather than leaving /run locks for reboot/tmpfs cleanup"
  ],
  "applyStatus": "not-implemented-in-this-build"
}
```

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered. | — |
| `2` | Unknown flag or invalid flag combination. | [`usage`](./error-codes.md#usage) |
| `78` | `--apply` or `--rollback` requested before the broker-backed mover is available. | `storage-migration-apply-not-implemented`, `storage-migration-rollback-not-implemented` |

**Human example**

```text
$ nixling host migrate-storage --dry-run
host migrate-storage --dry-run: checkpoint=storage-cutover-… vm_count=2
rollback command: nixling host migrate-storage --rollback --from-checkpoint storage-cutover-…
preflight requirements:
  - all nixling VMs stopped
  - nixlingd.service stopped
  - nixling-priv-broker.service stopped
  - net VMs stopped; guest routing, TAP connectivity, and dependent bridge traffic will be interrupted
```

**Native**

- `--dry-run` is a rust-native read-only planner.
- `--apply` and `--rollback` fail closed with typed exit-78 envelopes until the
  broker-backed mover lands. There is no bash fallback and no manual
  chmod/chown/setfacl remediation.

**Bash**

- No bash implementation exists. The Rust CLI owns this surface.

The dry-run text deliberately avoids manual `chmod`/`chown`/`setfacl`
instructions. Operators should treat the checkpoint ID and rollback command as
the handoff contract for the later broker-backed cutover implementation.

### `host reconcile-otel-acls` (reserved)

**Synopsis:** `nixling host reconcile-otel-acls [--dry-run | --apply] [--human | --json]`

Reconciles per-VM OTel relay/bridge filesystem ACLs to the
current bundle's intended state. Replaces the v0.4-era
`host-otel-relay-acl.nix` activation script (retired in v1.1
per [ADR 0018](../adr/0018-microvm-nix-removal.md) "Host-OTel
ACL migration table"). Invoked from
`system.activationScripts.nixlingReconcileOtelAcls` on every
`nixos-rebuild switch` and from `nixlingd.service` `ExecStartPost=`
on daemon startup; operators may also invoke it directly for
mid-cycle reconciliation.

**Behaviour**

- `--dry-run` (default if neither flag given): reports the
  planned ACL set/revoke ops without dispatching to the broker;
  exit 0 with a planned-ops summary.
- `--apply`: dispatches through `nixlingd` →
  `daemon-api/host-prep ReconcileOtelAcls` → broker
  `SetSocketAcl` / `RevokeSocketAclIfPresent` per the
  ADR 0018 migration table. Emits daemon-side
  `ReconcileOtelAclsStarted/Succeeded/Failed` `OpAuditRecord`
  entries with a `broker_op_ids` correlation array, plus the
  per-row broker-op entries.

**Exit codes / typed envelopes (per
[`docs/reference/error-codes.md`](./error-codes.md))**

- Exit 0: success (`--dry-run` plan summary OR `--apply` complete).
- Exit 1: `#daemon-down` (daemon unreachable; activation script
  defers to the daemon-startup `ExecStartPost=` trigger).
- Exit 31: `#broker-validation-failed` (broker rejected one or
  more `SetSocketAcl` / `RevokeSocketAclIfPresent` ops with a
  validation reason; see audit log for the per-op denial class
  per `docs/reference/error-codes.md:115`).
- Exit 50: `#internal-io` (broker dispatch error or daemon-side
  I/O failure during the reconcile).
- Exit 2: `#usage` (invalid flag combination).

**JSON output shape** (committed in v1.1 with full schema in
`cli-output/host-reconcile-otel-acls.schema.json`):

```json
{
  "mode": "dry-run|apply",
  "planned": [
    { "op": "SetSocketAcl", "path": "...", "group": "...", "mode": "rwx|--x|rw" },
    { "op": "RevokeSocketAclIfPresent", "path": "...", "groups": ["..."] }
  ],
  "applied": [],
  "broker_op_ids": ["..."],
  "daemon_op_id": "..."
}
```

(`applied` has the same shape as `planned`; it is populated only in
`--apply` mode and omitted/empty in `--dry-run` mode.)

**Native**

- v1.1 introduces this verb. v1.0 has no equivalent surface
  (the legacy `host-otel-relay-acl.nix` script is bash, not a
  daemon-dispatched CLI op).
- Daemon-unreachable surfaces `#daemon-down` exit 1 per the
  v1.1 typed-envelope contract.
- No bash fallback exists per [ADR 0017](../adr/0017-no-bash-fallbacks-invariant.md).

### `host doctor`

**Synopsis:** `nixling host doctor --read-only [--human] [--json]`

Read-only daemon-path health probe. The baseline broker socket, daemon
socket, and audit-log checks are extended with structured liveness for
the broker-spawned singletons and the recovery report files the daemon
persists during startup. Probed surfaces:

- **broker-ready** — `SOCK_SEQPACKET` connect to the broker socket.
- **daemon-ready** — connect to the public daemon socket.
- **metrics-endpoint** — loopback HTTP GET against the Prometheus scrape
  URL (default `http://127.0.0.1:9101/metrics`; see
  [`daemon-metrics`](./daemon-metrics.md)).
- **otel-host-bridge-runner** — counts `role: "otel-host-bridge"`
  entries in `<daemon-state>/pidfd-table.json`.
- **usbipd-runners** — counts `role: "usbip"` entries (one per env that
  owns USB).
- **kernel-module-matrix** — reads
  `<daemon-state>/kernel-module-report.json` (written by the daemon's
  startup self-check, see [`kernel-module-check`](./kernel-module-check.md)).
- **autostart-status** — reads `<daemon-state>/autostart-report.json`
  (written after the daemon's autostart pass, see
  [`daemon-autostart`](./daemon-autostart.md)).
- **storage-lifecycle-report** — reads
  `<daemon-state>/storage-lifecycle-report.json` (written by the daemon's
  startup storage/restart/sync contract check, see
  [`storage-lifecycle-report`](./storage-lifecycle-report.md)).

Doctor never calls a privileged broker operation. The `--read-only`
flag is currently mandatory; mutation forms are later deliverables. The
full JSON schema lives at
[`cli-output/host-doctor.schema.json`](./cli-output/host-doctor.schema.json);
prose is in [`cli-output/host-doctor.md`](./cli-output/host-doctor.md).
Legacy top-level fields (`broker_ready`, `findings[]`, `summary`,
`exitCode`) are preserved verbatim.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--read-only` | boolean | (required) | Acknowledge that this invocation does not mutate host state. Mandatory. |
| `--human` | boolean | `false` | Force the human doctor summary on stdout. |
| `--json` | boolean | `false` | Emit the stable doctor report document on stdout. |

**Environment overrides**

| Variable | Default | Purpose |
| --- | --- | --- |
| `NIXLING_BROKER_SOCKET` | `/run/nixling/broker.sock` | Probe target for `broker-ready`. |
| `NIXLING_PUBLIC_SOCKET` | `/run/nixling/public.sock` | Probe target for `daemon-ready`. |
| `NIXLING_DAEMON_STATE_DIR` | `/var/lib/nixling/daemon-state` | Where the daemon writes pidfd/module/autostart reports. |
| `NIXLING_METRICS_URL` | `http://127.0.0.1:9101/metrics` | URL probed by `metrics-endpoint`. |

**Exit codes**

| Exit | Meaning | Catalog anchor |
| --- | --- | --- |
| `0` | Every check passed. | — |
| `1` | At least one check is `warn`, none are `fail`. | [`host-check-warning`](./error-codes.md#host-check-warning) |
| `2` | At least one check is `fail` (e.g. required kernel module missing, autostart VM failed, broker socket unreachable). | [`host-check-warning`](./error-codes.md#host-check-warning) |
| `78` | Missing required `--read-only` flag (doctor is read-only; mutation forms are later deliverables). | [`--read-only-required`](./error-codes.md#--read-only-required) |

**Disposition:** `rust-native`.

### `host install`

**Synopsis:** `nixling host install (--dry-run | --apply [--enable] [--start | --no-start]) [--human | --json]`

`--dry-run` prints the synthesized 5-step installer preview. `--apply`
routes through the daemon → broker `RunHostInstall` path. Broker
failures surface the typed `broker-error` envelope with exit `78`;
they do **not** fall back to bash. If the daemon socket is
unreachable, the verb surfaces the typed `daemon-down` envelope with
exit `1` (the v1.0 daemon-only contract; the historical bash
fallback was retired in v1.0).

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Preview the synthesized 5-step install plan. |
| `--apply` | boolean | `false` | Run the live daemon → broker `RunHostInstall` path. |
| `--enable` | boolean | `false` | After `--apply`, enable `nixlingd.service`. |
| `--start` | boolean | `false` | After `--apply --enable`, start `nixlingd.service`. |
| `--no-start` | boolean | `false` | After `--apply`, leave `nixlingd.service` stopped. |
| `--json` | boolean | `false` | Emit the stable JSON plan or typed error envelope. |
| `--human` | boolean | `false` | Force the human summary. |

**Exit codes**

| Exit | Meaning | Catalog anchor |
| --- | --- | --- |
| `0` | Dry-run plan rendered or daemon → broker apply succeeded. | — |
| `78` | Missing `--dry-run` / `--apply`, or the daemon → broker apply path returned `broker-error`. | [`--apply-or-dry-run-required`](./error-codes.md#--apply-or-dry-run-required), [`broker-error`](./error-codes.md#broker-error) |

**Disposition:** `rust-native` (`--apply` dispatches through daemon → broker `RunHostInstall`).

### `migrate`

**Synopsis:** `nixling migrate (--dry-run | --apply) [--human | --json]`

`migrate` is the migration analyzer. `--dry-run` reports the current
deployment-shape tier plus the stable migration checklist. Per-VM
supervisor classification is still unavailable on the public manifest, so
the planner keeps that limitation explicit and points operators at
`nixling status <vm>` for per-VM truth. `--apply` uses the daemon-first
bridge and broker `RunMigrate` path.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Report the deployment-shape tier and the planned migration checklist. |
| `--apply` | boolean | `false` | Run the live daemon → broker `RunMigrate` marker-writer path. |
| `--json` | boolean | `false` | Emit the planner / apply result envelope on stdout. |
| `--human` | boolean | `false` | Force human-readable output on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Migration analysis is host-global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Missing explicit `--dry-run`/`--apply` or typed `broker-error` / `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`--apply-or-dry-run-required`](./error-codes.md#--apply-or-dry-run-required), [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling migrate --dry-run
nixling migrate --dry-run: deployment shape = tier-0-mixed, 2 VM(s) in manifest.
Per-VM supervisor classification is not available on the public manifest today.
Use `nixling status <vm>` to inspect each VM directly; `nixling migrate --apply`
is the live mutation path when you are ready.
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  [`not-yet-implemented`](./error-codes.md#not-yet-implemented) exit 78;
  [`broker-error`](./error-codes.md#broker-error) exit 78. The historical
  bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Native refusals stay on the typed envelope, and broker failures surface with exit `78`.
- Dry-run analysis is pure Rust; `--apply` dispatches through `nixlingd` →
  broker `RunMigrate`.

**Bash**

  `nixling migrate` path directly.
- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.

**Disposition:** `rust-native` — dry-run analysis is native, and
`--apply` uses daemon → broker `RunMigrate` when available.

### `auth status`

**Synopsis:** `nixling auth status [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the stable caller-capability document instead of the human summary. |
| `--human` | boolean | `false` | Force the human caller-capability summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Auth status is always global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Socket reachability or capability probe failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling auth status
role: launcher
public socket: /run/nixling/public.sock (reachable, server=0.2.0-w2, selected=0.2.0)
allowed commands: auth status, host check, list, status
denied commands:
- audit: requires admin role in nixling.site.adminUsers
```

**`--json` example** — schema: [`auth-status.schema.json`](./cli-output/auth-status.schema.json); prose companion: [`auth-status.md`](./cli-output/auth-status.md).

```json
{
  "role": "launcher",
  "publicSocket": {
    "path": "/run/nixling/public.sock",
    "reachable": true,
    "serverVersion": "0.2.0-w2",
    "selectedVersion": "0.2.0"
  },
  "allowedCommands": [
    "auth status",
    "host check",
    "list",
    "status"
  ],
  "deniedCommands": [
    {
      "command": "audit",
      "reason": "requires admin role in nixling.site.adminUsers"
    }
  ]
}
```

**Status**

Auth status is a read-only daemon query that reports caller mapping, socket reachability, and authorization hints.

**Native**

- Read-only daemon query resolving the caller's `SO_PEERCRED` role and the allowed/denied verb set.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `config sync`

**Synopsis:** `nixling config sync <vm> [--guest-path <path>] [--host <h>] [--user <u>] [--key <path>] [--known-hosts <path>] [--dry-run] [--json]`

<a id="config-sync-guest-control-transport"></a>
On a guest-control-capable VM, `config sync` reads the VM's canonical
guest config working copy (default `/var/lib/nixling-guest/guest-config.nix`)
over the authenticated **guest-control transport** — a typed
`readGuestConfig` request to `nixlingd` over the daemon public socket.
There is no SSH: no ssh/scp process is spawned, and the SSH-shaped flags
(`--host`/`--user`/`--key`/`--known-hosts`) and a non-default
`--guest-path` are rejected up front with `guest-control-ssh-flag-rejected`
(exit `2`). JSON responses carry `transport: "guest-control"`.

The host treats the received bytes as untrusted data: the read is bounded
(1 MiB hard cap + a per-attempt timeout, so a hostile guest cannot
OOM/hang the host), the host re-enforces the size cap and recomputes size
+ sha256 from the received bytes (never a guest-reported value), the
content is validated (non-empty, valid UTF-8), and the result is
atomically written to a host-side **staging** file
(`${XDG_STATE_HOME:-~/.local/state}/nixling/config-staging/<vm>.guest.nix`).
The staging copy is never evaluated until approved.

`--dry-run` selects and reports the transport WITHOUT contacting the
daemon or reading any guest bytes: it emits `transport: "guest-control"`
plus the planned staging target only — never an SSH argv and never guest
content.

Fail-closed behaviour:

- `config sync` is **admin-only**: `readGuestConfig` is gated to the
  `nixling-admin` role (`nixling.site.adminUsers`) at the daemon's
  `SO_PEERCRED` accept time. A launcher-role caller is rejected with the
  typed `authz-not-admin` error (exit `75`, AUTH) before any socket
  request reads guest bytes. The staging-only verbs (`diff`/`approve`/
  `reject`/`status`) dispatch no daemon verb and are not admin-gated.
- A known VM whose generation does not declare the guest-control transport
  (old or partial generation) is rejected with
  `guest-control-unavailable-old-generation` (exit `70`); no socket request
  is sent and nothing is staged. The operator SSH-compatibility transport
  is not wired into this command.
- When the daemon socket is unreachable, the command reports
  `guest-control-transport-unavailable` (exit `70`).
- Per-kind read errors surfaced by the daemon
  (`guest-control-file-not-found`, `guest-control-file-too-large`,
  `guest-control-path-unsafe`, `guest-control-read-denied`,
  `guest-control-timeout`, `guest-control-protocol-error`,
  `guest-control-auth-failed`, `guest-control-capability-unavailable`)
  each map to exit `70` with their slug and never echo guest content,
  paths, or transport detail.

**Disposition:** `rust-native` — host-initiated typed `readGuestConfig`
over the daemon public socket; no SSH, no virtiofs, no new privileged
surface.

### `config diff`

**Synopsis:** `nixling config diff <vm> --against <guestConfigFile> [--json]`

Shows a unified diff between the staged guest config and the live
host-side file the operator names with `--against` (typically their
`guestConfigFile`). Exits 0 whether or not they differ; `--json`
reports `differs` + the diff text.

**Disposition:** `rust-native` — read-only `diff -u`.

### `config approve`

**Synopsis:** `nixling config approve <vm> --to <target-file> [--json]`

Validates the staged guest config (non-empty, valid UTF-8) and
atomically writes it onto the operator-chosen `--to` target (unique
`O_EXCL` temp + fsync + rename + parent-dir fsync), then clears the
staging file. The CLI never auto-locates the operator's config tree —
the operator names the target explicitly. The authoritative containment
+ eval gate is the per-VM `guestConfigFile` assertion that runs on the
subsequent `nixling switch`.

**Disposition:** `rust-native` — host-operator-only; atomic publish.

### `config reject`

**Synopsis:** `nixling config reject <vm> [--json]`

Discards the staged guest config for a VM.

**Disposition:** `rust-native`.

### `config status`

**Synopsis:** `nixling config status [<vm>] [--all] [--json]`

Reports whether a VM (or, with `--all`, every VM) has a pending
(un-approved) staged guest config.

**Disposition:** `rust-native` — read-only.

### `config` exit codes + JSON envelopes

All `config` verbs share these exit codes:

| Exit | Meaning |
| --- | --- |
| `0` | Success (including `diff` whether or not files differ). |
| `1` | Runtime error: nothing staged, a low-level public-socket I/O failure on `config sync` (send/receive frame), size-cap/timeout on the staging verbs, missing `--to`/`--against` target dir, I/O error. |
| `2` | Usage error (bad/missing arguments; surfaced by `clap`), or `config sync` SSH-shaped flags rejected on a guest-control VM (`guest-control-ssh-flag-rejected`). |
| `70` | `config sync` only. The VM is not declared in the active manifest (`require_known_vm`); the VM's generation does not declare the guest-control transport (`guest-control-unavailable-old-generation`); the daemon socket is unreachable (`guest-control-transport-unavailable`); or a per-kind guest-control read error (`guest-control-file-not-found`, `guest-control-file-too-large`, `guest-control-path-unsafe`, `guest-control-read-denied`, `guest-control-timeout`, `guest-control-protocol-error`, `guest-control-auth-failed`, `guest-control-capability-unavailable`). The staging-only verbs (`diff`/`approve`/`reject`/`status`) do not consult the manifest or transport and so never return `70`. |
| `75` | `config sync` only. The caller is not in `nixling.site.adminUsers`. `config sync` dispatches the admin-only `ReadGuestConfig` daemon verb, so a launcher-role peer is rejected with the typed `authz-not-admin` (AUTH) error — exit `75`, the daemon's reserved authz code — before any guest read. The staging-only verbs (`diff`/`approve`/`reject`/`status`) dispatch no daemon verb and so never return `75`. |

With `--json` each verb emits a single stdout object:

- `config sync` → `{ "command": "config sync", "vm", "transport": "guest-control", "staging", "bytes", "sha256" }`
  (or `{ "command": "config sync", "mode": "dry-run", "vm", "transport": "guest-control", "staging", "guestFile" }` under `--dry-run` — no SSH argv, no guest bytes).
- `config diff` → `{ "command": "config diff", "vm", "against", "staging", "differs": <bool>, "diff": <string> }`.
- `config approve` → `{ "command": "config approve", "vm", "target", "bytes" }`.
- `config reject` → `{ "command": "config reject", "vm", "removed": <bool> }`.
- `config status` → `{ "command": "config status", "pending": [ <vm>… ] }`
  (the single-VM form reports a list with 0 or 1 entry).

Pending-staging notes (`nixling status`, `nixling up`/`start`, and the
mutating verbs) are emitted on **stderr** for human output only, so they
never perturb a `--json` stdout envelope.

### `vm exec`

**Synopsis:** `nixling vm exec [-i] [-t] [-d|--detach] [--env KEY=VALUE]… [--cwd DIR] [--json|--human] <vm> -- <cmd> [args…]`

**Detached management synopsis:**

- `nixling vm exec [--json] <vm> list`
- `nixling vm exec [--json] <vm> logs <exec-id>`
- `nixling vm exec [--json] <vm> status <exec-id>`
- `nixling vm exec [--json] <vm> kill <exec-id>`

Runs or manages commands inside a running VM over the authenticated
**guest-control transport**: the CLI opens an owner connection to the
daemon public socket, the daemon reaches the VM's `guestd` over the
authenticated guest-control vsock channel, and the endpoints exchange
typed `exec` operations. There is **no SSH** and **no host PTY** — the
guest owns the PTY. Exec is admin-only (the same `SO_PEERCRED` admin
gate as other privileged verbs); a launcher-role caller is rejected with
the typed `authz-not-admin` error (exit `77`, AUTH) before any guest
session or detached create is attempted.

**Status**

`vm exec` is a rust-native, daemon-backed guest command surface. It
supports attached, interactive, detached, and detached-management forms.

**Native**

- The Rust CLI owns parsing, terminal safety, detached-management
  rendering, and the single terminal JSON envelope contract.

**Bash**

- There is no live bash fallback for this verb; old-generation VMs fail
  closed with a typed guest-control error.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `-d`, `--detach` | boolean | `false` | Start a non-interactive detached exec and print its `exec_id`. Incompatible with `-i`/`-t`. |
| `-i`, `--interactive` | boolean | `false` | Forward host stdin. Must be paired with `-t`; `-t` implies stdin forwarding. |
| `-t`, `--tty` | boolean | `false` | Allocate a guest PTY and put the host terminal in raw mode. Human-only. |
| `--env` | `KEY=VALUE` | repeatable | Add one environment variable to the guest command after policy filtering. |
| `--cwd` | path | unset | Working directory for the guest command. |
| `--json` | boolean | `false` | Emit a single terminal JSON envelope. Human-only interactive modes reject this flag. |
| `--human` | boolean | `false` | Force human output. |
| `--stdout-offset` | byte offset | `0` | `logs` only. Resume retained stdout from this byte offset; accepts `--stdout-offset N` or `--stdout-offset=N`. |
| `--stderr-offset` | byte offset | `0` | `logs` only. Resume retained stderr from this byte offset; accepts `--stderr-offset N` or `--stderr-offset=N`. |
| `--max-len` | byte length | daemon default | `logs` only. Request at most this many retained bytes per stream; accepts `--max-len N` or `--max-len=N`. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `nixling.vms.<name>`. Management words such as `status` and `logs` are valid VM names because command execution always uses `--`. |
| `cmd [args…]` | Guest command and arguments after `--`. `argv[0]` may be a bare command name, absolute path, or relative path; it must be non-empty and must not start with `-`. |
| `list` | Detached management verb: list retained detached exec metadata. |
| `logs <exec-id>` | Detached management verb: emit retained stdout/stderr bytes plus bounded metadata warnings, optionally starting at per-stream offsets. |
| `status <exec-id>` | Detached management verb: print state, terminal disposition, and aggregate retained-log metadata. |
| `kill <exec-id>` | Detached management verb: request cancellation; repeated cancellation of a terminal exec reports `already-terminal`. |

Exec command forms **always require `--`** before `<cmd>`. Tokens after
`<vm>` without `--` are management verbs; an unknown verb is a usage
error that tells the operator to use `--` to run a command. This means
`list`, `logs`, `status`, and `kill` remain valid VM names:
`nixling vm exec list -- bash` runs `bash` in a VM named `list`, while
`nixling vm exec list status <id>` asks that VM for a detached exec's
status.

**Execution identity and command resolution.** Every attached and
detached exec runs the requested command as the VM's configured workload
user (`ssh.user`) — **never root** — inside a real PAM login session
(`systemd-run --property=PAMName=login --uid=<user>`). The command sees
the same login environment an interactive SSH login would
(`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, login-shell profile). `argv[0]`
may be a bare program name or relative path; the workload user's login
shell resolves it through that user's login `PATH` before `exec`. The
wire `user` field is host-fixed by `guestd` and ignored; operators
elevate with `sudo` inside the session. The console replacement is:

```console
$ nixling vm exec -it corp-vm -- bash
```

Modes:

- **Attached, non-interactive** (default): stdin is closed up front;
  stdout and stderr are streamed back as separate streams and written to
  the host's stdout/stderr.
- **`-t`/`--tty`**: allocates a PTY **in the guest**, puts the host
  terminal in raw mode for the session, and forwards host stdin to the
  guest command (`-t` implies `-i`). stderr is merged into stdout by the
  guest PTY. Requires stdin **and** stdout to be a terminal. Interactive
  modes are human-only: `-t` (and `-i`) are rejected together with
  `--json`.
- **`-i`/`--interactive`**: forwards host stdin to the guest command
  (non-blocking, partial-write aware) until EOF, which closes the guest
  stdin. The guest-control transport forwards stdin **only** in PTY
  mode, so `-i` **must** be paired with `-t` (use `-it`); `-i` without
  `-t` is a usage error.
- **`-d`/`--detach`**: creates a non-interactive detached exec, prints
  the opaque `exec_id`, and returns without streaming stdio. The command
  continues after the host client disconnects. `-d` is rejected with
  `-i` or `-t`, and it still requires a command after `--`.

Attached FSM (one session, one exec, no per-op reconnect): the CLI
drains enqueued host signals, forwards ready stdin, then
bounded-long-polls stdout (and, in non-tty mode, polls stderr) so stdin
and signals are never starved behind an output poll. When both streams
reach EOF it polls `Wait` until the terminal disposition is known,
having already flushed all output. Host terminal state (termios +
`O_NONBLOCK`) is restored on **every** exit, error, disconnect, or panic
via an RAII guard.

Attached signal forwarding (enqueue-only; handlers never touch termios
or make syscalls): `SIGWINCH` → guest PTY `Resize` (tty mode only);
`SIGINT` → guest signal `2`; `SIGQUIT` → `3`; `SIGHUP` → `1`;
`SIGTERM` → `15`; `SIGTSTP` → `20`, all delivered to the exec's
foreground process group.

Detached management:

- **create** — `nixling vm exec -d <vm> -- <cmd> [args…]`: human output
  is one copy-pasteable `exec_id` line. JSON emits
  `{ "command": "vm exec", "vm": "<vm>", "execId": "<id>", "state": "<state>" }`.
- **list** — `nixling vm exec <vm> list`: human output is a table with
  `execId`, state, start time, terminal status when available, aggregate
  and per-stream retained offset windows, and aggregate/per-stream
  dropped/truncated metadata. JSON
  emits `{ "command": "vm exec list", "vm": "<vm>", "execs": [ { "execId",
  "state", "startedAt", "exitCode"?, "signal"?, "startOffset",
  "endOffset", "droppedBytes", "truncated" } ] }`; implementations also
  expose per-stream stdout/stderr offsets and dropped/truncated flags for
  resume-capable clients.
- **status** — `nixling vm exec <vm> status <exec-id>`: human output is
  the state plus terminal disposition. JSON emits
  `{ "command": "vm exec status", "vm": "<vm>", "execId": "<id>",
  "state", "reason"?, "exitCode"?, "signal"?, "startOffset",
  "endOffset", "droppedBytes", "truncated" }`.
- **logs** — `nixling vm exec <vm> logs <exec-id>`: human output writes
  retained stdout/stderr bytes to the corresponding host streams and
  prints only bounded metadata warnings to stderr when bytes were
  dropped or truncated. An expired detached record is a typed failure
  (`guest-control-exec-expired`, exit `76`), not a warning. JSON emits
  `{ "command": "vm exec logs", "vm": "<vm>", "execId": "<id>",
  "stdoutBase64", "stderrBase64", "startOffset", "endOffset",
  "droppedBytes", "truncated" }`, plus per-stream
  `stdoutStartOffset`/`stdoutEndOffset`/`stdoutNextOffset`/`stdoutEof`
  and `stderrStartOffset`/`stderrEndOffset`/`stderrNextOffset`/
  `stderrEof` fields for offset resume. Logs are bounded ring buffers;
  dropped and truncated
  accounting is metadata, not log content.
- **kill** — `nixling vm exec <vm> kill <exec-id>`: public name for
  `ExecCancel`. Guestd requests graceful termination, waits a bounded
  grace window, then force-kills the workload if needed. The operation is
  idempotent: human output confirms the result, and JSON emits
  `{ "command": "vm exec kill", "vm": "<vm>", "execId": "<id>",
  "result": "cancelling"|"already-terminal", "state": "<state>" }`.

Detached exec is supervised inside `guestd` and its in-guest detached
runner. It does not add a privileged broker operation. Guestd reconciles
detached runner/workload units before advertising detached capability,
re-adopts structurally valid work, cleans orphaned workload units, and
runs a periodic reaper for terminal records and retained-log slots.

**Exit codes**

| Exit | Source | Meaning |
| --- | --- | --- |
| `0` | cli | Detached create/list/status/logs/kill succeeded. |
| `0`–`255` | guest | Attached guest command `WIFEXITED` status passes through unchanged. |
| `128+N` | guest | An attached guest command was killed by signal `N` (`WIFSIGNALED`). |
| `2` | cli / guest-control | Usage error: missing command after `--`, unknown management verb without `--`, missing detached exec id, malformed `--env`, `-d` with `-i`/`-t`, `-t` without a terminal, `-i` without `-t`, `--json` combined with `-i`/`-t`, or guest-side `INVALID_PROGRAM` (`guest-control-invalid-program`) for an empty/leading-`-` program. |
| `69` | transport | The guest-control transport was unreachable, a per-op/establishment deadline elapsed, or `guestd` disappeared before the exec reported a terminal status (`guest-control-transport-unavailable`, `guest-control-timeout`, `guest-control-lost-guestd`). |
| `70` | guest-control | The VM generation does not support guest-control exec, or it lacks a required exec capability (`guest-control-unavailable-old-generation`, `guest-control-capability-unavailable`, `guest-control-exec-detached-unavailable`). No SSH fallback. |
| `75` | guest-control | The exec session table is at capacity, `Start` was rate limited, or an established session was cancelled/reaped before a terminal guest status arrived (`exec-session-capacity`, `exec-session-rate-limited`, `exec-session-cancelled`, `exec-session-reaped`). |
| `76` | protocol / guest-control | The guest returned a malformed/out-of-contract response, returned an op error, or no longer retains the requested detached record (`guest-control-protocol-error`, `guest-control-exec-error`, `guest-control-exec-not-found`, `guest-control-exec-expired`). |
| `77` | guest-control | The authenticated guest-control handshake was rejected (`guest-control-auth-failed`), the daemon's admin gate refused a non-admin caller (`authz-not-admin`), or a stale exec session was detected (`guest-control-stale-session`). |
| `42` | internal | Daemon-internal or CLI-internal failure driving exec. |

**Human example**

```text
$ nixling vm exec work -- id
uid=1000(alice) gid=100(users)
$ nixling vm exec work list
EXEC ID                  STATE                  STARTED AT                EXIT/SIGNAL    OFFSETS                                    DROPPED/TRUNCATED
exec-1                   exited                 2026-06-15T00:00:00Z      exit=0         all=4..18 stdout=4..8 stderr=9..18         all=5/truncated stdout=2/truncated stderr=3/complete
$ nixling vm exec work logs exec-1 --stdout-offset=4 --stderr-offset=9 --max-len=4096
OUT
ERR
nixling: vm exec logs: retained output incomplete (startOffset=4 endOffset=18 droppedBytes=5 truncated=true stdoutStartOffset=4 stdoutEndOffset=8 stdoutNextOffset=10 stdoutEof=false stdoutDroppedBytes=2 stdoutTruncated=true stderrStartOffset=9 stderrEndOffset=18 stderrNextOffset=21 stderrEof=true stderrDroppedBytes=3 stderrTruncated=false)
```

Detached JSON shapes are generated as
[`vm-exec-create.schema.json`](./cli-output/vm-exec-create.schema.json),
[`vm-exec-list.schema.json`](./cli-output/vm-exec-list.schema.json),
[`vm-exec-status.schema.json`](./cli-output/vm-exec-status.schema.json),
[`vm-exec-logs.schema.json`](./cli-output/vm-exec-logs.schema.json), and
[`vm-exec-kill.schema.json`](./cli-output/vm-exec-kill.schema.json).

**`--json` example**

```json
{
  "command": "vm exec logs",
  "vm": "work",
  "execId": "exec-1",
  "stdoutBase64": "T1VUCg==",
  "stderrBase64": "RVJSCg==",
  "startOffset": 4,
  "endOffset": 18,
  "droppedBytes": 5,
  "truncated": true,
  "stdoutStartOffset": 4,
  "stdoutEndOffset": 8,
  "stdoutNextOffset": 10,
  "stdoutEof": false,
  "stdoutDroppedBytes": 2,
  "stdoutTruncated": true,
  "stderrStartOffset": 9,
  "stderrEndOffset": 18,
  "stderrNextOffset": 21,
  "stderrEof": true,
  "stderrDroppedBytes": 3,
  "stderrTruncated": false
}
```

A guest command that itself exits `70` (or any reserved transport
number) is **not** ambiguous in machine-readable output: attached
`--json` carries `source` plus `guestExitCode`/`transportExitCode` so a
consumer distinguishes a guest exit code from a transport class that
shares the shell status number.

**Attached `--json` envelope** (non-interactive, non-detached only): a
single terminal stdout object.

- success → `{ "command": "vm exec", "vm", "source": "guest", "exitCode", "reason": "exited"|"signaled", "guestExitCode"?|"signal"?, "stdoutBase64", "stderrBase64", "stdoutTruncated", "stderrTruncated" }`. Only a true guest `WIFEXITED`/`WIFSIGNALED` terminal is a success.
- failure → `{ "command": "vm exec", "vm", "source": "transport"|"guest-control"|"protocol"|"internal"|"cli", "reason": "<wire-kind>", "exitCode", "transportExitCode"?, "message", "remediation"? }`. Abnormal terminal kinds (`lost-guestd`, `cancelled`, `reaped`) and a malformed/missing terminal status are failures with a reserved code and a non-`guest` source — never a synthesized guest exit. A failure envelope never carries captured stdio bytes. Usage errors (`source: "cli"`, exit `2`) also emit one envelope.

Captured output in JSON envelopes is bounded; `stdoutTruncated` /
`stderrTruncated` flag a clamp. argv, env, cwd, and stdio bytes never
appear in any span, log, audit record, or metric label. Attached exec
emits only an aggregate outcome counter and a single kind=critical
session-establishment event (VM name, peer uid, negotiated tty).
Detached create/kill daemon audit is similarly redacted: VM, peer uid,
closed action/result enums, and the opaque exec id only.

**Disposition:** `rust-native` — daemon public socket → authenticated
guest-control session → `guestd` exec RPCs; no SSH, no host PTY, no new
privileged broker op (attached sessions live in-process in `nixlingd`;
detached state lives in guestd's detached registry).

## Dispatch capability table

| Command | Current disposition | Rationale |
| --- | --- | --- |
| `list` | `rust-native` | Pure read-only inventory query; the daemon answers it without mutating host or guest state. |
| `vm start` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed `SpawnRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm stop` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed `SignalRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm restart` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed stop+start sequence. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm list` | `rust-native` | Daemon-side runtime inventory from nixlingd's public socket; daemon-unavailable returns an explicit empty inventory with remediation text. |
| `status` | `rust-native` | Status is a read-only daemon RPC, including the frozen per-VM JSON shape. |
| `status --check-bridges` | `rust-native` | The bridge-health probe is part of the read-only status surface, even though reconcile remains deferred. |
| `usb attach` | `rust-native` | USBIP attach parses and dispatches one intent to `nixlingd`; the daemon coordinates broker host bind/firewall/proxy state and authenticated guestd import over guest-control. |
| `usb detach` | `rust-native` | USBIP detach parses and dispatches one intent to `nixlingd`; the daemon asks guestd to detach matching imports, then runs broker `UsbipUnbind` / `UsbipProxyReconcile`. |
| `usb probe` | `rust-native` | USBIP probe is a read-only daemon query backed by the broker's `UsbipProxyReconcile` validation pass. |
| `console` | `rust-native shim` | The Rust CLI owns help / argument validation; the daemon-native foreground console handoff is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017). Today the verb surfaces the typed `not-yet-implemented` envelope (exit `78` per ADR 0015). |
| `audio status` | `rust-native shim` | The Rust CLI owns help / argument validation; the daemon-native audio-status surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017). Today the verb surfaces the typed `not-yet-implemented` envelope (exit `78` per ADR 0015). |
| `audio mic` | `rust-native shim` | The Rust CLI owns help / argument validation; the daemon-native microphone grant surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017). Today the verb surfaces the typed `not-yet-implemented` envelope (exit `78` per ADR 0015). |
| `audio speaker` | `rust-native shim` | The Rust CLI owns help / argument validation; the daemon-native speaker grant surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017). Today the verb surfaces the typed `not-yet-implemented` envelope (exit `78` per ADR 0015). |
| `audio off` | `rust-native shim` | The Rust CLI owns help / argument validation; the daemon-native `off` shorthand is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017). Today the verb surfaces the typed `not-yet-implemented` envelope (exit `78` per ADR 0015). |
| `build` | `rust-native` | Build is a native non-destructive planner that renders the eval/build preview without falling back to bash. |
| `switch` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `boot` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `test` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `rollback` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `generations` | `rust-native` | Generations is a native introspection surface that reports current/booted symlink targets without falling back to bash. |
| `gc` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunGc` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `store verify` | `rust-native` | Routes through `nixlingd` → broker `StoreVerify`; the CLI never reads the store-view directly. |
| `trust` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunHostKeyTrust` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `rotate-known-host` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunRotateKnownHost` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `keys list` | `rust-native` | Keys list is a native inventory preview that reports the managed-key resolution placeholders without falling back to bash. |
| `keys show` | `rust-native` | Keys show is a native preview that reports daemon-resolved key metadata placeholders without falling back to bash. |
| `keys rotate` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunKeysRotate` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `audit` | `rust-native` | Audit is part of the daemon surface and keeps both human and JSON output contracts. |
| `host check` | `rust-native` | Host check is a read-only daemon RPC by design. |
| `host prepare` | `rust-native` | The Rust CLI owns dry-run output (wired live); `--apply` is **not yet wired** — it returns the typed `daemon-down` envelope (exit `1`) today (use `--dry-run` for now). When the daemon-side dispatch ships, `--apply` will route through the daemon-backed `ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `UpdateHostsFile` / `ApplyNmUnmanaged` sequence, with broker failures surfacing `broker-error` (exit `78`); a Tier-0 host is refused today (exit `78`). The historical bash fallback was retired in v1.0. |
| `host destroy` | `rust-native` | The Rust CLI owns dry-run output (wired live); `--apply` is **not yet wired** — it returns the typed `daemon-down` envelope (exit `1`) today (use `--dry-run` for now). When the daemon-side dispatch ships, `--apply` will route through the reverse-order daemon-backed host-reconcile sequence, with broker failures surfacing `broker-error` (exit `78`); a Tier-0 host is refused today (exit `78`). The historical bash fallback was retired in v1.0. |
| `host doctor` | `rust-native` | Host doctor is a read-only daemon health probe; `--read-only` is mandatory and there is no bash fallback for mutation forms. |
| `host migrate-storage` | `rust-native` | Storage cutover dry-run planning is native and read-only; `--apply` / `--rollback` fail closed until the broker-backed mover lands. |
| `host install` | `rust-native` | Host install owns its dry-run preview in Rust and routes `--apply` through the daemon → broker `RunHostInstall` path without broker-error fallback to bash. |
| `migrate` | `rust-native` | Dry-run analysis is native; `--apply` routes through `nixlingd` → broker `RunMigrate`. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `auth status` | `rust-native` | Auth status is a read-only daemon query that reports caller mapping, socket reachability, and authorization hints. |
| `vm exec` | `rust-native` | Daemon public socket → authenticated guest-control session → `guestd` exec RPCs. Admin-only; no SSH, no host PTY, no new privileged broker op. Attached exec uses the in-process `nixlingd` session table; detached exec uses guestd's detached registry and VM-first management verbs. |
