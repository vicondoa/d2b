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
| `1` | Unexpected local probe or manifest-read failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ nixling list
NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
corp-vm            work      false     false false   10.20.0.10      stopped
sys-work-net       work      false     false false   192.0.2.1       stopped (net-vm)
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
    "status": "stopped",
    "isNetVm": false
  },
  {
    "name": "sys-work-net",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "192.0.2.1",
    "status": "stopped",
    "isNetVm": true
  }
]
```

**Disposition:** `rust-native` — Pure read-only inventory query; the daemon can answer it without mutating host or guest state.

Rows are ordered by VM name because the historical bash implementation iterated `jq keys[]`; the current daemon-native path keeps that ordering contract.
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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

**Status:** `vm list` is the reserved daemon-side runtime inventory surface,
but the current CLI keeps the stable shape explicit and still returns a
placeholder empty inventory until live runner enumeration is wired through
this command. Use `nixling status <vm>` for per-VM runtime truth today.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the stable placeholder inventory document on stdout. |
| `--human` | boolean | `false` | Force the human placeholder summary on stdout. |

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
vm list: daemon runner inventory not yet exposed here; use `nixling status <vm>`
```

**`--json` example**

```json
{
  "command": "vm list",
  "entries": [],
  "notes": "vm list placeholder: live daemon runner inventory is not wired through this surface yet; use `nixling status <vm>` for per-VM truth."
}
```

**Current disposition:** `rust-native` placeholder — the Rust CLI owns the
stable daemon-side runtime-view contract here, but the live runner table is
not wired through this surface yet.

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
  "services": {
    "nixling": "inactive",
    "microvm": "inactive",
    "virtiofsd": "inactive",
    "gpu": null,
    "snd": null,
    "swtpm": null
  },
  "current": null,
  "booted": null,
  "pendingRestart": false,
  "apiReady": null
}
```

**Disposition:** `rust-native` — Status is a read-only daemon RPC,
including the frozen per-VM JSON shape. Guest-control rollout will add a
negotiated guest-control state field in the implementation wave; it must
not appear as an ad hoc unversioned key.

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

**Disposition:** `rust-native` — The bridge-health probe is part of the read-only status surface, even though reconcile remains deferred.

### `usb attach`

**Synopsis:** `nixling usb attach <vm> <busid> [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the daemon → broker USBIP attach plan without mutating host state. |
| `--apply` | boolean | `false` | Ask `nixlingd` to run `UsbipBind` (acquiring the per-busid lock and validating ownership), apply the USBIP firewall carve-out, ensure the per-env USBIP backend/proxy runners are ready, run `UsbipProxyReconcile` for the selected VM/busid pair, then SSH into the guest and run `sudo -n usbip attach -r <usbipdHostIp> -b <busid>`. |
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
nixling usb attach --dry-run: would bind and lock, apply the USBIP firewall carve-out, ensure the per-env backend/proxy for busid '1-2' for vm 'corp-vm', and reconcile the USBIP proxy
```

**Disposition:** `rust-native` — The native CLI drives the daemon → broker `UsbipBind`, `UsbipBindFirewallRule`, per-env backend/proxy ensurement, and `UsbipProxyReconcile` path directly, then performs the guest-side `usbip attach` over the framework-managed SSH key.

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
nixling usb detach --dry-run: would unbind busid '1-2' for vm 'corp-vm' and reconcile the USBIP proxy
```

**Disposition:** `rust-native` — The native CLI drives the daemon → broker `UsbipUnbind` / `UsbipProxyReconcile` path directly.

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

**Disposition:** `rust-native` — Probe is a read-only daemon RPC backed by the broker's `UsbipProxyReconcile` validation pass.

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

**Disposition:** `rust-native shim` — The Rust CLI owns help and argument validation, but returns a typed exit-78 envelope in v1.0 (daemon-native console surface queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015).

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

**Disposition:** `rust-native shim` — The Rust CLI owns help and argument validation, but returns a typed exit-78 envelope in v1.0 (daemon-native audio surface queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015).

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

**Disposition:** `rust-native shim` — The Rust CLI owns help and argument validation, but returns a typed exit-78 envelope in v1.0 (daemon-native audio hotplug surface queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015).

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

**Disposition:** `rust-native shim` — The Rust CLI owns help and argument validation, but returns a typed exit-78 envelope in v1.0 (daemon-native audio speaker surface queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015).

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

**Disposition:** `rust-native shim` — The Rust CLI owns help and argument validation, but returns a typed exit-78 envelope in v1.0 (daemon-native audio off surface queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017); see ADR 0015).

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

**Disposition:** `rust-native` — Build is a native non-destructive planner that renders the eval/build preview without falling back to bash.
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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

**Disposition:** `rust-native` — Generations is a native introspection surface that reports current/booted symlink targets without falling back to bash.
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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

**Disposition:** `rust-native` — Keys list is a native inventory preview that reports the managed-key resolution placeholders without falling back to bash.

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

**Disposition:** `rust-native` — Keys show is a native preview that reports daemon-resolved key metadata placeholders without falling back to bash.
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
| `70` | The named VM is not declared in the active manifest. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
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

**Disposition:** `rust-native` — Audit is part of the read-only daemon surface and keeps both human and JSON output contracts.

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

**Disposition:** `rust-native` — Host check is a read-only daemon RPC by design; mutation is explicitly handled by host prepare.

The command never mutates nftables, cgroups, users, or runtime directories. `--read-only` is therefore part of the compatibility surface, not a capability toggle.
### `host prepare`

**Synopsis:** `nixling host prepare [--dry-run | --apply] [--human | --json]`

**Status**

`host prepare` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "nixling: NOTICE: defaulting to --dry-run; nixling 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the host reconcile without mutating host state. |
| `--apply` | boolean | `false` | Perform the host-reconcile mutation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Host reconcile is global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run summary rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Tier-0 all-legacy refusal, Tier-0 mixed single-writer conflict, or typed `broker-error` / `not-yet-implemented` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`tier-0-legacy-uses-nixos-module`](./error-codes.md#tier-0-legacy-uses-nixos-module), [`single-writer-conflict`](./error-codes.md#single-writer-conflict), [`broker-error`](./error-codes.md#broker-error) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling host prepare --dry-run
host prepare --dry-run: would reconcile nftables + routes + sysctls + /etc/hosts + NetworkManager unmanaged state
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `nixlingd` → broker `ApplyNftables` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNmUnmanaged` (commit `ee6ed0b`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `host destroy`

**Synopsis:** `nixling host destroy [--dry-run | --apply] [--human | --json]`

**Status**

`host destroy` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "nixling: NOTICE: defaulting to --dry-run; nixling 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`; `--apply` is daemon-native.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan removal of nixling-owned host reconcile state without mutating host state. |
| `--apply` | boolean | `false` | Perform the host-reconcile teardown. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Host teardown is global. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run summary rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | Tier-0 all-legacy refusal or typed `broker-error` (v1.0 daemon-only per ADR 0015; no bash fallback). | [`tier-0-legacy-uses-nixos-module`](./error-codes.md#tier-0-legacy-uses-nixos-module), [`broker-error`](./error-codes.md#broker-error) |

In v1.0 daemon-only (per ADR 0015) the historical bash fallback was retired in v1.0; the verb surfaces typed envelopes (`broker-error` exit 78, `daemon-down` exit 1) instead.

**Human example**

```text
$ nixling host destroy --dry-run
host destroy --dry-run: no nixling-owned resources to remove
```

**Native**

- `--apply`: routes through `nixlingd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: same broker-op set in reverse order: `ApplyNmUnmanaged` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNftables` (commit `ee6ed0b`; reverse-order hardening in `b73e28f`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


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

**Disposition:** `rust-native` — Auth status is a read-only daemon query that reports caller mapping, socket reachability, and authorization hints.

### `config sync`

**Synopsis:** `nixling config sync <vm> [--guest-path <path>] [--host <h>] [--user <u>] [--key <path>] [--known-hosts <path>] [--dry-run] [--json]`

Pulls the VM's in-guest edited `guestConfigFile` (default
`/var/lib/nixling-guest/guest-config.nix`) over the framework-managed
per-VM SSH key into a host-side **staging** file
(`${XDG_STATE_HOME:-~/.local/state}/nixling/config-staging/<vm>.guest.nix`).
The host treats the pulled bytes as untrusted data: the pull is bounded
(1 MiB hard cap + 120 s timeout, so a hostile guest cannot OOM/hang the
host), validated (non-empty, valid UTF-8), and the staging copy is never
evaluated until approved. The VM's host key is verified against
`--known-hosts` (default `/var/lib/nixling/known_hosts.nixling`) with
`StrictHostKeyChecking=accept-new` (pins on first use; refuses a changed
key). The SSH user comes from `--user` or the manifest `ssh_user`
(set `nixling.vms.<vm>.ssh.user`); there is no `$USER` fallback.
`--dry-run` prints the SSH command without running it.

**Disposition:** `rust-native` — host-initiated SSH copy; reuses the
existing per-VM key + manifest `static_ip` / `ssh_user`. No new
privileged surface, no virtiofs.

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
| `1` | Runtime error: nothing staged, SSH failure, size-cap/timeout, missing `ssh.user`, missing `--to`/`--against` target dir, I/O error. |
| `2` | Usage error (bad/missing arguments; surfaced by `clap`). |
| `70` | `config sync` only: the VM is not declared in the active manifest (`require_known_vm` emits the typed `not-yet-implemented` host-error envelope). The staging-only verbs (`diff`/`approve`/`reject`/`status`) do not consult the manifest and so never return `70`. |

With `--json` each verb emits a single stdout object:

- `config sync` → `{ "command": "config sync", "vm", "staging", "bytes" }`
  (or `{ …, "mode": "dry-run", "argv", "staging" }` under `--dry-run`).
- `config diff` → `{ "command": "config diff", "vm", "against", "staging", "differs": <bool>, "diff": <string> }`.
- `config approve` → `{ "command": "config approve", "vm", "target", "bytes" }`.
- `config reject` → `{ "command": "config reject", "vm", "removed": <bool> }`.
- `config status` → `{ "command": "config status", "pending": [ <vm>… ] }`
  (the single-VM form reports a list with 0 or 1 entry).

Pending-staging notes (`nixling status`, `nixling up`/`start`, and the
mutating verbs) are emitted on **stderr** for human output only, so they
never perturb a `--json` stdout envelope.

## Dispatch capability table

| Command | Current disposition | Rationale |
| --- | --- | --- |
| `list` | `rust-native` | Pure read-only inventory query; the daemon answers it without mutating host or guest state. |
| `vm start` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed `SpawnRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm stop` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed `SignalRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm restart` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed stop+start sequence. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm list` | `rust-native` placeholder | Reserves the daemon-side runtime-view contract, but today returns an explicit empty inventory until live runner enumeration is wired through this surface. |
| `status` | `rust-native` | Status is a read-only daemon RPC, including the frozen per-VM JSON shape. |
| `status --check-bridges` | `rust-native` | The bridge-health probe is part of the read-only status surface, even though reconcile remains deferred. |
| `usb` | `rust-native` | USBIP attach/detach/probe now parse and dispatch through the native daemon path. |
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
| `trust` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunHostKeyTrust` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `rotate-known-host` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunRotateKnownHost` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `keys list` | `rust-native` | Keys list is a native inventory preview that reports the managed-key resolution placeholders without falling back to bash. |
| `keys show` | `rust-native` | Keys show is a native preview that reports daemon-resolved key metadata placeholders without falling back to bash. |
| `keys rotate` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunKeysRotate` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `audit` | `rust-native` | Audit is part of the daemon surface and keeps both human and JSON output contracts. |
| `host check` | `rust-native` | Host check is a read-only daemon RPC by design. |
| `host prepare` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `UpdateHostsFile` / `ApplyNmUnmanaged` sequence. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `host destroy` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the reverse-order daemon-backed host-reconcile sequence. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `host doctor` | `rust-native` | Host doctor is a read-only daemon health probe; `--read-only` is mandatory and there is no bash fallback for mutation forms. |
| `host install` | `rust-native` | Host install owns its dry-run preview in Rust and routes `--apply` through the daemon → broker `RunHostInstall` path without broker-error fallback to bash. |
| `migrate` | `rust-native` | Dry-run analysis is native; `--apply` routes through `nixlingd` → broker `RunMigrate`. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `auth status` | `rust-native` | Auth status is a read-only daemon query that reports caller mapping, socket reachability, and authorization hints. |
