# d2b CLI contract

**Diataxis category:** reference.

This document is the command contract for the single user-facing
`d2b` entry point. It covers the CLI surfaces that are fully
owned in Rust, including the read-only and daemon-backed commands
that go through `d2bd`.

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
> for the multi-line block format used on those envelopes. `d2b
> up/down/restart/list` are first-class top-level aliases for `vm
> start/stop/restart/list` and route through the same daemon path.
> Stop-like aliases accept the same `--force` / `-f` flag as the
> corresponding `vm` command.

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

## Realm target boundary

Realm-aware target strings use the canonical form
`<workload>.<realm>[.<ancestor>...].d2b`. Bare workload names are aliases only
when a caller supplies an alias table or default realm, and old node-qualified
forms are diagnostics that point to the realm-native target with the node label
removed. See [Realm access resolver contract](./realm-access-resolver.md) for
the target grammar, direct host-local socket binding, capability preflight, and
typed denial shapes.

VM lifecycle and arbitrary exec remain VM-only. A trusted unsafe-local target is
rejected before those handlers can coerce its workload id to a VM name.
Provider-neutral list, status, and configured launch use the workload operation
family on the local `d2bd` public socket.

### Configured launcher operation

The provider-neutral command is:

```text
d2b launch <canonical-target> [--item <item-id>]
```

The public request carries only the canonical target, configured item id, and
an idempotency operation id. It never carries argv, uid, environment, cwd,
display paths, process ids, or unit names. An `exec` item dispatches through the
selected provider. A local-VM `shell` item dispatches existing persistent-shell
semantics. An unsafe-local `shell` item requires `unsafe-local-shell-v1` and
invokes `d2b shell` with the workload's canonical target; there is no host-shell
or SSH fallback. When `--item` is omitted, the CLI selects `defaultItem`, then
an only item, otherwise returns the available item ids and names.

For local-VM exec items, d2bd derives an opaque guest exec id from the
authenticated requester, operation id, target, and item id. Guestd persists that
id with the detached exec record, so replay after a daemon restart returns the
existing exec instead of spawning a duplicate. A replay whose trusted argv hash
does not match fails closed.

The DTOs remain protocol version 3 and are gated by `configured-launch-v1`.
Unsafe-local additionally requires `unsafe-local-provider-v1`; shell items also
require `unsafe-local-shell-v1`. Unsupported peers return an update remediation
and never fall back.

**Exit codes**

| Code | Meaning |
| --- | --- |
| `0` | Launch committed or was already committed for the operation id. |
| `2` | Target/item not found, or omitted item is ambiguous. |
| `31` / `75` | Caller lacks launcher/admin authority or the operation is temporarily busy. |
| `69` | Provider prerequisite or transport unavailable. |
| `70` | Capability unavailable, provider mismatch, or required feature/version skew. |
| `76` | Protocol response or operation-id conflict. |

## Command reference

### `launch`

**Synopsis:** `d2b launch <TARGET> [--item <ITEM>] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--item <ITEM>` | string | unset | Select the configured launcher item id. When omitted, use `defaultItem`, then a sole item, otherwise return the available ids and names. |
| `--json` | boolean | `false` | Emit the stable machine-readable launch result on stdout. |
| `--human` | boolean | `false` | Force the human launch confirmation on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `TARGET` | Canonical workload target or an unambiguous workload id. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Launch committed or was already committed. | — |
| `2` | Target/item not found, or omitted item is ambiguous. | [`usage`](./error-codes.md#usage) |
| `31` / `75` | Caller lacks launcher/admin authority or the operation is temporarily busy. | workload launch error |
| `69` | Provider prerequisite or transport unavailable. | workload launch error |
| `70` | Capability unavailable, provider mismatch, or required feature/version skew. | workload launch error |
| `76` | Protocol response or operation-id conflict. | workload launch error |

**Human example**

```text
$ d2b launch tools.host.d2b --item browser
launched tools.host.d2b item browser (committed)
```

**`--json` example** — schema: [`launch.schema.json`](./cli-output/launch.schema.json).

```json
{
  "command": "launch",
  "target": "tools.host.d2b",
  "itemId": "browser",
  "operationId": "launch-1234-5678",
  "disposition": "committed"
}
```

### `list`

**Synopsis:** `d2b list [--human] [--json]`

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
$ d2b list
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
    "isNetVm": false,
    "guestClosureOutPath": "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-nixos-system-corp-vm"
  },
  {
    "name": "sys-work-net",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "usbip": false,
    "staticIp": "192.0.2.1",
    "status": "running",
    "isNetVm": true,
    "guestClosureOutPath": "/nix/store/ffffffffffffffffffffffffffffffff-nixos-system-sys-work-net"
  }
]
```

**Status**

`list` is a daemon-native, read-only inventory query. When d2bd is
reachable, the CLI queries the public socket and reports declared VM
metadata, daemon-derived lifecycle state, and the VM guest closure out
path when closure metadata is available. For a running VM with
`status = "pending-restart"`, `guestClosureOutPath` points at the
booted closure so scanners inspect the running guest generation. If the
public socket is unavailable or does not support the request, the CLI
falls back to the static manifest/local status path and may still
populate `guestClosureOutPath` when the caller can read local closure
metadata.

**Native**

- Pure read-only public-socket inventory query; no broker op and no guest
  contact. The static manifest/local status path is fallback only.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

Rows are ordered by VM name because the historical bash implementation iterated `jq keys[]`; the current daemon-native path keeps that ordering contract.

### `clipboard arm`

**Synopsis:** `d2b clipboard arm [--human | --json]`

Opens the d2b clipboard picker for the current host-focused Niri target.
After the operator selects an item, `d2b-clipd` publishes that item as a
d2b-owned host selection and triggers paste replay for the focused target.
If the picker cannot be launched or its handshake fails, `d2b-clipd` reports a
typed failure instead of silently writing clipboard data.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit one JSON response on stdout. Failures are structured as `{ "ok": false, "error": "<bounded-message>" }`. |
| `--human` | boolean | `false` | Force human text on stdout. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | The picker opened, or picker launch/handshake failed and `d2b-clipd` successfully armed the native paste fallback. | — |
| `2` | The control socket was unavailable, malformed, timed out, or returned a daemon error. | [`usage`](./error-codes.md#usage) |

The CLI connects to `$XDG_RUNTIME_DIR/d2b-clipd/clipd.sock`, sends one bounded
arm request, and applies five-second read and write deadlines to the
control socket so a wedged `d2b-clipd` cannot hang the terminal. The
daemon owns all clipboard state and transfer FDs; this command only asks
the daemon to open the picker for d2b-owned paste replay.

**Human examples**

```text
$ d2b clipboard arm
picker opened
$ d2b clipboard arm
picker_not_configured
```

**`--json` examples**

```json
{ "ok": true, "message": "picker opened" }
```

```json
{ "ok": false, "error": "failed to connect to clipboard daemon: No such file or directory (os error 2)" }
```

**Native**

- Rust-native local user-session control-socket request to `d2b-clipd`.
  No broker op, no guest contact, no shell-out, no synthetic input, and
  no clipboard payload or transfer FD crosses the CLI boundary.

### `realm enter`

**Synopsis:** `d2b realm enter <realm>`

Enters the local gateway VM for a gateway-backed realm by opening an
interactive `vm exec` session to the gateway workload. The host resolves
the realm through the generated realm entrypoint table and verifies the
gateway VM is declared and running before attempting the exec. Realm
relay/provider credentials remain inside the gateway guest.

### `realm list`

**Synopsis:** `d2b realm list [--human | --json]`

Lists rendered local realm entrypoints. The output reports each realm's mode
(`host-resident` or `gateway-backed`), gateway VM when present, local gateway
lifecycle state, credential boundary, and default-deny cross-realm policy.

### `realm inspect`

**Synopsis:** `d2b realm inspect <realm> [--human | --json]`

Inspects one realm entrypoint using the same bounded fields as `realm list`.
Unknown realms fail closed with the same actionable missing-entrypoint envelope
used by routed VM targets.

### `op inspect`

**Synopsis:** `d2b op inspect [--trace-id <id> --span-id <id>] [--human | --json]`

Inspects current local constellation operation state without making the host a
global telemetry owner. The command reports bounded local VM/gateway counts,
configured realm states, optional trace context, and degraded partial results
for unavailable gateways or sinks. It never falls back to SSH, host-held realm
credentials, or generic tunnels.

### `realm run`

**Synopsis:** `d2b realm run <realm> -- <argv...> [--human | --json]`

Runs a one-shot command inside the local gateway VM for a
gateway-backed realm. This is the low-level escape hatch for scripts that
need to issue an exact command in the realm trust boundary. Daily
workload operations should continue to use `d2b vm ...`; realm
targets route through the configured gateway entrypoint when supported.

### Realm target routing

The realm target grammar is
`<workload>.<realm>[.<ancestor>...].d2b`. Bare local VM names stay on the
existing host fast path until the runtime/Nix cutover lands. Fully qualified
realm targets must resolve through the realm access layer; missing entrypoints
fail closed with an actionable `missing-realm-entrypoint` error rather than
falling back to SSH or a generic tunnel.

`d2b vm list --realm <realm>` runs `d2b vm list` inside the
realm gateway through the same local guest-control exec path. It does not
make the host persist a remote node/workload registry.

### `vm display`

**Synopsis:**

- `d2b vm display list [--target <workload>.<realm>.d2b] [--human | --json]`
- `d2b vm display close <session-id> [--human | --json]`

`vm display` manages active gateway display sessions. It requires the
gateway daemon's public socket and does not fall back to SSH or host-side
Wayland setup. `list` returns only bounded non-secret session metadata:
session id, realm target, lifecycle state, authorizing operation id, and
principal. For launcher-role callers, `list` returns only sessions owned by
the caller's local socket uid and `close` can tear down only those sessions;
admin callers can inspect or close any active display session. `close` asks
the gateway daemon to tear down the listener plus the provider-side display
agent when the session is still active, and reports `closed = false` for an
already-absent session.

`--json` for `list` emits:

```json
{
  "command": "vm display list",
  "target": "demo.work.d2b",
  "sessions": [
    {
      "sessionId": "s0",
      "target": "demo.work.d2b",
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

**Synopsis:** `d2b vm start <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`vm start` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `d2b: NOTICE: defaulting to
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
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `d2b up` exit table is preserved in this
file as history.

**Human example**

```text
$ d2b vm start corp-vm --apply
vm start corp-vm: spawned pid=4242 start_time_ticks=123456789
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: `d2bd` → broker `SpawnRunner`.

**Bash**

- There is no bash execution path for this verb.
### `vm stop`

**Synopsis:** `d2b vm stop <vm> [--force | -f] [--dry-run | --apply] [--human | --json]`

**Status**

`vm stop` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `d2b: NOTICE: defaulting to
--dry-run` and the CLI defaults to `--dry-run`; `--apply` routes
through the daemon.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the 5-node per-VM DAG without spawning any role. |
| `--apply` | boolean | `false` | Perform the lifecycle mutation. |
| `--force`, `-f` | boolean | `false` | Skip provider-aware graceful guest shutdown and begin the standard SIGTERM/SIGKILL VMM cleanup path. This is not an immediate SIGKILL shortcut. |
| `--json` | boolean | `false` | Emit the dry-run DAG or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `d2b down` exit table is preserved in this
file as history.

Normal `vm stop --apply` asks supported local providers to shut the guest
down before host-side VMM cleanup: Cloud Hypervisor receives
`PUT /api/v1/vm.shutdown`, and qemu-media receives broker-mediated QMP
`system_powerdown`. The wait is bounded by
`d2b.daemon.lifecycle.gracefulShutdown.timeoutSeconds` or the per-VM
manifest override. Human output prints progress such as
`Waiting for guest to shut down (up to 90s)...`; the maximum command wait is
that graceful timeout plus the standard forced-cleanup signal windows.
`--force` / `-f` bypasses only the graceful wait.

Pidfd `EPERM` while stopping a per-VM-UID runner used to surface as
typed `broker-error` exit 78. Current `--apply` recovers that specific
case by asking the broker to run `SignalRunner`; if the broker reports
`signaled=true`, `vm stop` exits 0. True broker failures — unreachable
broker, dispatch errors, unexpected responses, or `signaled=false` —
still surface as `broker-error` / exit 78.

**Human example**

```text
$ d2b vm stop corp-vm --apply
Waiting for guest to shut down (up to 90s)...
vm stop corp-vm: clean guest shutdown
```

**Native**

- `--apply`: routes through `d2bd` → provider graceful shutdown
  (when enabled) → broker cleanup as needed. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `--force` / `-f`: available on both `vm stop` and the top-level
  `down` alias. It is an explicit stop override, not a shortcut around
  the existing forced cleanup policy.
- `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: `d2bd` → CH API or broker-mediated QMP for guest
  shutdown → broker `SignalRunner` / cgroup cleanup fallback.

**Bash**

- There is no bash execution path for this verb.
### `vm restart`

**Synopsis:** `d2b vm restart <vm> [--force | -f] [--dry-run | --apply] [--human | --json]`

**Status**

`vm restart` is a daemon-native headless-lifecycle verb. If neither
mutation flag is set, stderr emits `d2b: NOTICE: defaulting to
--dry-run` and the CLI defaults to `--dry-run`; `--apply` routes
through the daemon.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the 5-node per-VM DAG without spawning any role. |
| `--apply` | boolean | `false` | Perform the lifecycle mutation. |
| `--force`, `-f` | boolean | `false` | Apply force only to the stop phase: skip graceful guest shutdown, then run the usual cleanup before the unchanged start phase. |
| `--json` | boolean | `false` | Emit the dry-run DAG or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Typed `broker-error` or `not-yet-implemented`. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

There is no bash fallback. Daemon-unreachable returns `daemon-down`
(exit 1), and the old `d2b restart` exit table is preserved in
this file as history.

**Human example**

```text
$ d2b vm restart corp-vm --apply
vm restart corp-vm: vm stop corp-vm: clean guest shutdown; vm start corp-vm: spawned pid=4242 start_time_ticks=123456789
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  `not-yet-implemented` exit 78; `broker-error` exit 78.
- `--force` / `-f`: available on both `vm restart` and the top-level
  `restart` alias. It affects only the stop phase.
- `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` are
  unrecognised. Broker failures surface on stderr with the redacted
  public-safe remediation and exit `78`.
- Live path: same as `vm stop` for the stop phase, then broker
  `SpawnRunner` for the start phase.

**Bash**

- There is no bash execution path for this verb.

### `vm list`

**Synopsis:** `d2b vm list [--human] [--json]`

**Status:** `vm list` is the daemon-side runtime inventory surface. It
queries d2bd's public socket and returns the same live lifecycle/runtime
entries the daemon exposes to desktop clients. If the public socket is not
available, the command exits successfully with an empty `entries` array plus
a note explaining that d2bd must be started or restarted.

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
$ d2b vm list
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
        "d2b": "active",
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

When d2bd's public socket is unavailable, `--json` returns:

```json
{
  "command": "vm list",
  "entries": [],
  "notes": "vm list requires d2bd's public socket; start or restart d2bd and retry."
}
```

**Current disposition:** `rust-native` — the Rust CLI owns the stable
daemon-side runtime-view contract and reads it from d2bd's public socket.

### `status`

**Synopsis:** `d2b status [<vm>] [--json]`

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
$ d2b status corp-vm
=== corp-vm ===
daemon: inactive
backend-runner: inactive
virtiofsd: inactive
gpu-runner: stopped
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
    "d2b": "inactive",
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

**Synopsis:** `d2b status --check-bridges`

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
$ d2b status --check-bridges
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

**Synopsis:** `d2b usb attach <vm> <busid> [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the daemon → broker USBIP attach plan plus the authenticated guestd import step without mutating host or guest state. |
| `--apply` | boolean | `false` | Ask `d2bd` to run three fail-closed pre-flight checks (sysfs presence, USB-capable gate, active claim exclusivity), then dispatch the appropriate broker path: **declared path** (when a static bundle intent exists for the busid — `UsbipBind` + firewall carve-out + `UsbipProxyReconcile`), or **explicit path** (when no declared intent exists — `UsbipExplicitFirewallRule` + `UsbipExplicitBind` per-device ops), then ask guestd over authenticated guest-control to import the selected busid. |
| `--json` | boolean | `false` | Emit the dry-run summary as structured JSON. |
| `--human` | boolean | `false` | Force the human dry-run summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name. |
| `busid` | Required host USB busid in the canonical `B-P[.P...]` form (for example `1-2` or `2-1.4`). Does not require the busid to be pre-declared in the NixOS bundle configuration. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `d2bd` is unreachable, or the daemon returned a non-typed USBIP failure. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Missing VM / busid or another usage error. | [`usage`](./error-codes.md#usage) |
| `67` | The USB device busid is not present in sysfs (`usbip-busid-not-present`), or another VM already holds an active claim on this busid (`usbip-explicit-claim-conflict`). | [`usbip-busid-not-present`](./error-codes.md#usbip-busid-not-present), [`usbip-explicit-claim-conflict`](./error-codes.md#usbip-explicit-claim-conflict) |
| `78` | The daemon reached the broker but the native USBIP apply path was refused. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ d2b usb attach corp-vm 1-2 --dry-run
d2b usb attach --dry-run: would bind and lock, apply the USBIP firewall carve-out, ensure the per-env backend/proxy for busid '1-2' for vm 'corp-vm', reconcile the USBIP proxy, and ask guestd to import the device
```

**Explicit attach (present-busid, no static allowlist required)**

`d2b usb attach` accepts any USB device that is physically present in sysfs
without requiring a static busid or vendor allowlist in the NixOS configuration.
The daemon selects the **explicit path** when no declared bundle intent exists for
the requested busid. Three fail-closed checks run before any broker call:

1. **Sysfs presence** — the daemon checks `/sys/bus/usb/devices/<busid>/idVendor`.
   If absent, the attach fails with `UsbipBusidNotPresent` (exit 67) and guides
   the operator to plug in the device before retrying.
2. **USB-capable gate** — the VM must have `RuntimeCapabilityGate::UsbHotplug`
   declared in its manifest. Non-USB-capable VMs fail with a typed
   `RuntimeCapabilityUnsupported` error.
3. **Active claim exclusivity** — the daemon reads the OFD lock under
   `/run/d2b/locks/usbip/<busid>`. If another VM already holds the claim, the
   attach fails with `UsbipExplicitClaimConflict` (exit 67) naming the owner VM
   and guiding the operator to detach from the owner first.

The explicit path dispatches `UsbipExplicitFirewallRule` (env-scoped nftables
rule keyed on the per-env uplink IPs) and `UsbipExplicitBind` (per-device backend
setup). Both ops are currently typed stubs; the live per-device backend handler
is deferred to later implementation. The declared path (static bundle intents) is
unaffected.

**Status**

The native CLI sends one intent to `d2bd`; the daemon drives broker host
USBIP state and authenticated guestd import cleanup/attach over guest-control.
If the target VM is stopped, `--apply` fails before host mutation with an
actionable usage error: start the VM with
`d2b vm start <vm> --apply`, wait until it is running, then retry
`d2b usb attach <vm> <busid> --apply`. This preflight does not create a
degraded USB state. If an earlier failed apply left a stale or bound USBIP
session claim, start the VM and rerun the attach or run
`d2b usb detach <vm> <busid> --apply`; the attach/detach paths and
`d2b usb probe` all run the USBIP proxy reconcile pass, and `usb probe`
shows the session claim as cleared once the lock/proxy state is consistent.

Prerequisites for `--apply` are: the target VM is running and guest-control
advertises USBIP status/import, the bundle declares a USBIP bind/firewall intent
for the VM/busid, policy/topology checks allow the physical device, the
`usbip-host` module and per-env backend/proxy carrier can be prepared, and no
other owner holds the busid session claim. Failing prerequisites surface as
typed errors or as `d2b usb probe` degraded reasons with exact remediation
commands.

**Native**

- `--apply` routes through `d2bd` → broker + guestd. There is no SSH fallback for USBIP.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb detach`

**Synopsis:** `d2b usb detach <vm> <busid> [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the daemon → broker USBIP unbind plan without mutating host state. |
| `--apply` | boolean | `false` | Ask `d2bd` to run `UsbipUnbind` followed by `UsbipProxyReconcile` for the selected VM/busid pair. |
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
| `1` | `d2bd` is unreachable, or the daemon returned a non-typed USBIP failure. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Missing VM / busid or another usage error. | [`usage`](./error-codes.md#usage) |
| `78` | The daemon reached the broker but the native USBIP apply path was refused. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ d2b usb detach corp-vm 1-2 --dry-run
d2b usb detach --dry-run: would ask guestd to detach busid '1-2' for vm 'corp-vm', unbind it on the host, and reconcile the USBIP proxy
```

**Status**

The native CLI first asks guestd to detach matching imports, then drives the
daemon → broker `UsbipUnbind` / `UsbipProxyReconcile` path. Explicit detach is
the only normal path that releases a USBIP session claim. VM stop/restart keeps
the claim for the same VM within the current host boot/session so restart
reconciliation can re-import the device; a host reboot clears the `/run` lock.

Single-busid detach never stops the shared per-env proxy. If the daemon cannot
prove firewall-withdrawal-before-flow-kill ordering plus an exact VM/proxy
cleanup tuple, `--apply` fails closed with `usbip-revocation-not-isolated` and
preserves the session claim for manual drain/recovery. The public error names the
target busid. The safe next step is to stop the VM so the stream drains, then
rerun `d2b usb detach <vm> <busid> --apply`.

**Native**

- `--apply` routes through `d2bd` → broker `UsbipUnbind` then `UsbipProxyReconcile`.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb probe`

**Synopsis:** `d2b usb probe [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured USBIP probe inventory instead of the human table. |
| `--human` | boolean | `false` | Force the human USBIP probe table on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | The probe always lists every daemon-declared USBIP busid session claim. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | `d2bd` is unreachable or does not expose the native USBIP probe request. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `78` | The daemon reached the broker but the `UsbipProxyReconcile` pass failed. | [`broker-error`](./error-codes.md#broker-error) |

**Human example**

```text
$ d2b usb probe
VM                       ENV          BUSID        STATUS     SESSION-CLAIM          HOST-BIND                CARRIER        PROXY        GUEST      POLICY
corp-vm                  work         1-2          degraded   held-by-desired-owner  unknown                  unknown        unknown      detached   allowed
  degraded guest-import-unavailable: the guest USBIP import has not converged
  remediation: Run `d2b usb attach corp-vm 1-2 --apply` after the VM is running.
  command: d2b usb attach corp-vm 1-2 --apply
```

**`--json` example** — schema: [`usb-probe.schema.json`](./cli-output/usb-probe.schema.json); prose companion: [`usb-probe.md`](./cli-output/usb-probe.md).

```json
{
  "command": "usb probe",
  "entries": [
    {
      "vm": "corp-vm",
      "env": "work",
      "busId": "1-2",
      "lockPath": "/run/d2b/locks/usbip/1-2",
      "status": "degraded",
      "ownerVm": "corp-vm",
      "durableClaim": {
        "state": "held-by-desired-owner",
        "ownerVm": "corp-vm"
      },
      "host": {
        "bind": "unknown",
        "carrier": "unknown",
        "proxy": "unknown"
      },
      "guest": {
        "import": "detached"
      },
      "topologyPolicy": {
        "topology": "unknown",
        "policy": "allowed"
      },
      "degradedReasons": [
        {
          "code": "guest-import-unavailable",
          "summary": "the guest USBIP import has not converged",
          "remediation": "Run `d2b usb attach corp-vm 1-2 --apply` after the VM is running."
        }
      ],
      "remediationCommands": [
        "d2b usb attach corp-vm 1-2 --apply"
      ]
    }
  ]
}
```

**Status**

Probe is a read-only daemon RPC backed by the broker's
`UsbipProxyReconcile` validation pass. The JSON and human forms split:

- the **session claim** (`missing`, `held-by-desired-owner`,
  `held-by-other-owner`, `stale-owner`, `corrupt`, or `not-applicable`);
- **active host carrier** state for module/device/backend readiness
  (`absent`, `unavailable`, `withheld-for-owner`, `ready`,
  `departed-during-probe`, `unknown`, or `not-applicable`);
- host driver bind, per-env proxy listener, guest import, and redacted
  topology/policy state;
- closed degraded reasons with human remediation; and
- copy-paste lifecycle commands such as
  `d2b usb attach corp-vm 1-2 --apply` or
  `d2b usb detach <owner> 1-2 --apply` when the daemon can name a safe
  next command.

A persisted lock by itself is not reported as healthy `bound`; stale or
incomplete lock-only state is `degraded` until host and guest state are
reconciled. Public output uses redacted state labels and summaries rather
than raw sysfs paths, raw serials, stderr, or policy internals.

VM restart reconciliation preserves same-VM session claims for the current host
boot/session, detaches guest imports, and only runs host unbind after firewall
withdrawal plus targeted stream cleanup is proven. It then replays host
bind/proxy and guest import after guest-control readiness. Runtime absence,
proxy unavailability, or guest import unavailability surfaces as degraded USB
state. Required policy/topology failures fail before device exposure and must be
remediated by fixing the declaration, rebuilding, and rerunning the lifecycle
command.

**Native**

- Read-only daemon query enumerating every declared USBIP busid session claim.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `usb security-key status`

**Synopsis:** `d2b usb security-key status [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the structured security-key proxy status as JSON. |
| `--human` | boolean | `false` | Force human-readable output on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Always returns the full proxy status: configured keys, per-VM virtual-device health, current lease, and USBIP conflict state. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `78` | The daemon handler for this command has not shipped yet. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

**Human example**

```text
$ d2b usb security-key status
# not yet available — exits 78 with not-yet-implemented envelope
```

**Status**

This command is defined and CLI-stable. The live (non-dry-run) path exits 78
with a `not-yet-implemented` envelope until the security-key proxy daemon
handler ships. Use `d2b usb security-key test <vm> --dry-run` to preview
the planned checks.

Use the user-facing term "security key" in user-visible text; FIDO/CTAP
terminology is reserved for diagnostics and technical documentation.

### `usb security-key sessions`

**Synopsis:** `d2b usb security-key sessions [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--json` | boolean | `false` | Emit the session list as JSON. |
| `--human` | boolean | `false` | Force human-readable output. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| _(none)_ | Returns all recent and active security-key request sessions. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `78` | The daemon handler for this command has not shipped yet. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

**Human example**

```text
$ d2b usb security-key sessions
# not yet available — exits 78 with not-yet-implemented envelope
```

**Status**

CLI-stable. Exits 78 with a `not-yet-implemented` envelope until the
security-key proxy daemon handler ships.

### `usb security-key cancel`

**Synopsis:** `d2b usb security-key cancel {<session-id> | --current} [--dry-run | --apply] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--current` | boolean | `false` | Cancel the currently active session. Mutually exclusive with `<session-id>`. |
| `--dry-run` | boolean | `false` | Print the planned broker op without dispatching it. |
| `--apply` | boolean | `false` | Dispatch the cancel through the daemon → broker path. |
| `--json` | boolean | `false` | Emit structured output as JSON. |
| `--human` | boolean | `false` | Force human-readable output on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `<session-id>` | Optional. Opaque session ID returned by `sessions`. Mutually exclusive with `--current`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success (`--dry-run` or successful cancel). | — |
| `2` | Neither `<session-id>` nor `--current` was provided; or neither `--dry-run` nor `--apply` was provided. | [`usage`](./error-codes.md#usage) |
| `78` | `--apply`: the daemon handler has not shipped yet. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

**Human example**

```text
$ d2b usb security-key cancel --current --dry-run
d2b usb security-key cancel --dry-run: would send CancelSession(current) to the security-key proxy broker
```

**`--json` example**

```json
{
  "command": "usb security-key cancel",
  "mode": "dry-run",
  "notes": "Dry-run preview; --apply dispatches the cancel through the daemon → broker SecurityKeyProxyCancelSession path.",
  "planned": [
    "SecurityKeyProxyCancelSession"
  ],
  "target": "current"
}
```

**Status**

`--dry-run` is fully implemented and golden-stable. `--apply` exits 78 until
the daemon handler ships.

### `usb security-key test`

**Synopsis:** `d2b usb security-key test <vm> [--dry-run] [--human] [--json]`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | `false` | Print the planned checks without contacting the daemon. |
| `--json` | boolean | `false` | Emit structured output as JSON. |
| `--human` | boolean | `false` | Force human-readable output on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `<vm>` | Required. VM name with the per-VM USB security-key option enabled. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success (`--dry-run` or all checks passed). | — |
| `78` | Live path: the daemon handler has not shipped yet. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

**Human example**

```text
$ d2b usb security-key test corp-vm --dry-run
d2b usb security-key test --dry-run: would check virtual HID device presence in 'corp-vm' and confirm host broker sees the physical security key
```

**`--json` example**

```json
{
  "command": "usb security-key test",
  "mode": "dry-run",
  "notes": "Dry-run preview; the live path queries the daemon for virtual-HID presence in the guest and physical-key visibility on the host broker.",
  "planned": [
    "CheckGuestVirtualHidDevice",
    "CheckHostBrokerPhysicalKeyVisibility"
  ],
  "vm": "corp-vm"
}
```

**Status**

`--dry-run` is fully implemented and golden-stable. The live path exits 78
until the daemon handler ships.

### `console`

**Synopsis:** `d2b console <vm>`

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| _(none)_ | — | — | Serial console access has no command-line flags. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required headless VM name. Graphics VMs are rejected and must be launched with `d2b vm start <vm> --apply`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Success. | — |
| `1` | Console launch or output read failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM, missing argument, or graphics VM selected. | [`usage`](./error-codes.md#usage) |
| `80` | `provider-misconfigured`: ACA sandbox without an active guestd-compatible console transport; see [ACA console — provider misconfiguration](./provider-capability-matrix.md#aca-console--provider-misconfiguration). | [`provider-misconfigured`](./error-codes.md#provider-misconfigured) |

**Human example**

```text
$ d2b console corp-vm
Connected to console for VM 'corp-vm' (LocalHypervisor). Press Ctrl-] to detach.
```

Console control messages are emitted on stderr. Stdout is reserved for the raw
guest UART byte stream so `d2b console <vm> > console.log` captures only guest
output.

**Status**

The Rust CLI dispatches `ConsoleOp` to `d2bd` over the public socket. The daemon owns a persistent ring-buffer drainer per VM and hands the attached operator session reads from that buffer. Provider-capability resolution runs before attach; ACA targets without an active guestd-compatible terminal transport surface a typed `provider-misconfigured` error (exit `80`) rather than falling back to any shell channel. See [provider capability matrix](./provider-capability-matrix.md) for the per-provider transport model; the design is governed by [ADR 0041](../adr/0041-console-and-audio-controls.md) and [ADR 0015](../adr/0015-daemon-only-clean-break.md).

**Native**

- Parses and validates arguments natively, then dispatches `ConsoleOp` to `d2bd` over the public socket; surfaces typed error envelopes on failure.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio status`

**Synopsis:** `d2b audio status [<vm>]`

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
| `0` | Success. Per-target `enforcement: unsupported` (e.g. qemu-media guest-side) is reported in the output body, not as an error exit. | — |
| `1` | Unexpected filesystem or state probe failure. | [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `80` | `provider-misconfigured`: ACA sandbox without an active guestd audio transport; see [provider capability matrix](./provider-capability-matrix.md#aca-audio). | [`provider-misconfigured`](./error-codes.md#provider-misconfigured) |

**Human example**

```text
$ d2b audio status corp-vm
audio:    enabled
mic:      off
speaker:  off
sidecar:  inactive
device:   detached
```

**Status**

The Rust CLI dispatches `AudioOp::GetState` to `d2bd` over the public socket. Provider capability resolution runs before any state access; Cloud Hypervisor NixOS VMs read OFD-locked state from `/run/d2b/audio/<vm>.json`, qemu-media VMs report `enforcement: unsupported` for guest-side, and ACA sandbox VMs route through provider guestd. Multi-target queries return per-target errors so one misconfigured provider does not fail the entire response. See [provider capability matrix](./provider-capability-matrix.md) for the per-provider enforcement model.

**Native**

- Parses and validates arguments natively, then dispatches `AudioOp::GetState` to `d2bd` over the public socket; surfaces typed error envelopes on failure.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio mic`

**Synopsis:** `d2b audio mic on|off <vm>`

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
| `0` | Success. State is persisted and enforcement applied; `applied: host-only` is reported only for providers such as qemu-media where guest enforcement is unsupported. | — |
| `1` | Audio state write, sidecar, or hotplug failure. | [`generic`](./error-codes.md#generic) |
| `2` | Bad state literal, unknown VM, or audio not enabled for the VM. | [`usage`](./error-codes.md#usage) |
| `80` | `provider-misconfigured`: ACA sandbox without an active guestd audio transport. | [`provider-misconfigured`](./error-codes.md#provider-misconfigured) |

**Human example**

```text
$ d2b audio mic on corp-vm
d2b audio: state -> mic=on, speaker=off

audio:    enabled
mic:      on
speaker:  off
sidecar:  active
device:   will-attach-on-next-up
```

**Status**

The Rust CLI dispatches `AudioOp::SetMic` to `d2bd` over the public socket. The daemon writes OFD-locked state atomically and, where guest enforcement is supported, applies the guest mic grant via guestd before reporting `host-and-guest`. Providers without guest enforcement, such as qemu-media, report the explicit `host-only` posture. See [provider capability matrix](./provider-capability-matrix.md) for the per-provider enforcement model.

**Native**

- Parses and validates arguments natively, then dispatches `AudioOp::SetMic` to `d2bd` over the public socket; surfaces typed error envelopes on failure.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio speaker`

**Synopsis:** `d2b audio speaker on|off <vm>`

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
| `0` | Success. State is persisted and enforcement applied; `applied: host-only` is reported only for providers such as qemu-media where guest enforcement is unsupported. | — |
| `1` | Audio state write, sidecar, or hotplug failure. | [`generic`](./error-codes.md#generic) |
| `2` | Bad state literal, unknown VM, or audio not enabled for the VM. | [`usage`](./error-codes.md#usage) |
| `80` | `provider-misconfigured`: ACA sandbox without an active guestd audio transport. | [`provider-misconfigured`](./error-codes.md#provider-misconfigured) |

**Human example**

```text
$ d2b audio speaker on corp-vm
d2b audio: state -> mic=off, speaker=on

audio:    enabled
mic:      off
speaker:  on
sidecar:  active
device:   will-attach-on-next-up
```

**Status**

The Rust CLI dispatches `AudioOp::SetSpeaker` to `d2bd` over the public socket. Behavior mirrors `audio mic`: state is persisted atomically under OFD lock and guest enforcement is applied where available. See [provider capability matrix](./provider-capability-matrix.md) for the per-provider enforcement model.

**Native**

- Parses and validates arguments natively, then dispatches `AudioOp::SetSpeaker` to `d2bd` over the public socket; surfaces typed error envelopes on failure.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `audio off`

**Synopsis:** `d2b audio off <vm>`

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
| `0` | Success. Calling the command against a VM that never had audio enabled is an idempotent no-op. `applied: host-only` is reported only for providers such as qemu-media where guest enforcement is unsupported. | — |
| `1` | Audio state write or sidecar failure. | [`generic`](./error-codes.md#generic) |
| `2` | Missing or unknown VM name. | [`usage`](./error-codes.md#usage) |
| `80` | `provider-misconfigured`: ACA sandbox without an active guestd audio transport. | [`provider-misconfigured`](./error-codes.md#provider-misconfigured) |

**Human example**

```text
$ d2b audio off corp-vm
d2b audio: state -> mic=off, speaker=off

audio:    enabled
mic:      off
speaker:  off
sidecar:  inactive
device:   detached
```

**Status**

The Rust CLI dispatches `AudioOp::Mute` to `d2bd` over the public socket. The daemon atomically revokes both mic and speaker grants, persisting state under OFD lock and applying guest enforcement where available. See [provider capability matrix](./provider-capability-matrix.md) for the per-provider enforcement model.

**Native**

- Parses and validates arguments natively, then dispatches `AudioOp::Mute` to `d2bd` over the public socket; surfaces typed error envelopes on failure.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.

### `build`

**Synopsis:** `d2b build <vm>`

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
$ d2b build corp-vm
d2b: building corp-vm closure...
d2b: corp-vm closure → /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-nixos-system-corp-vm
  GC root: /var/lib/d2b/vms/corp-vm/result
```

**Status**

Build is a native non-destructive planner that renders the eval/build preview without falling back to bash.

**Native**

- Native eval/build planner; renders the closure preview and GC-root path.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.
### `switch`

**Synopsis:** `d2b switch <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`switch` is a daemon-native live guest activation verb. If neither
mutation flag is set, the CLI prints the parity notice and defaults to
`--dry-run`; `--apply` requires the VM to be running and reachable over
guest-control.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Publish the prepared VM closure into the VM's live store pool, ask guestd to activate that prepared toplevel inside the running guest, poll guest activation status, then commit the successful generation through the broker. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `1` | `d2bd` is unreachable or guest-control transport fails before a typed activation result is available. | [`daemon-down`](./error-codes.md#daemon-down), [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Host store publication, broker commit, guest activation capability, or guest activation status failed closed. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

Stopped/offline VMs fail closed: `switch --apply` never asks the host
broker to execute the guest activation program, never mutates host
systemd for the VM, and never falls back to SSH or a host shell. Start
the VM first with `d2b vm start <vm> --apply`, wait for guest-control
readiness, then rerun `d2b switch <vm> --apply`.

The live guest activation wait is bounded by
`d2b.daemon.lifecycle.liveActivation.timeoutSeconds` or the per-VM
`d2b.vms.<vm>.lifecycle.liveActivation.timeoutSeconds` override. If
activation times out in an identity-bound guest, the typed error points at
the guest activation unit. Complete the in-guest provider flow (for example
an Entra/Himmelblau hello/PIN prompt) and retry, or use `d2b boot <vm>
--apply` followed by a VM restart when live user-session activation is
expected to block.

**Human example**

```text
$ d2b switch corp-vm --apply
d2b switch --apply activated in guest via guest-control (vm=corp-vm, mode=switch, summary=activated, generationNumber=42)
```

**Native**

- `--apply`: routes through `d2bd`, which prepares/publishes the
  closure, opens the authenticated guest-control activation flow, waits
  for guestd status, and only then asks the broker to commit host-side
  generation metadata. Successful commits publish both legacy activation
  metadata and split store-view `state/current` / `meta/current` pointers.
  Daemon-unreachable surfaces `daemon-down` exit 1;
  guest capability/readiness and broker failures surface typed non-zero
  envelopes. There is no host-side execution of guest activation scripts.
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars
  were retired; the daemon-only contract is the only path.

**Bash**

- There is no live bash fallback for this verb.
### `boot`

**Synopsis:** `d2b boot <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`boot` is the daemon-native offline staging activation verb. If neither
mutation flag is set, the CLI prints the parity notice and defaults to
`--dry-run`; `--apply` does not require a running guest.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the offline staging update without mutating guest or host generation state. |
| `--apply` | boolean | `false` | Publish the prepared VM closure into the VM's live store pool and commit it as the toplevel to use on the next VM start; no guest-control activation is attempted. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `1` | `d2bd` is unreachable. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Host store publication, broker commit, or native handler support failed closed. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

`boot --apply` is the explicit way to stage a new toplevel while the VM
is stopped/offline. It only changes the host-side generation selected
for the next start; it does not run guest activation in the current
guest and does not require guest-control capability.

**Human example**

```text
$ d2b boot corp-vm --apply
d2b boot --apply staged next-boot toplevel (vm=corp-vm, mode=boot, summary=staged for next boot, generationNumber=42)
```

**Native**

- `--apply`: routes through `d2bd` and the broker-backed store
  publication/commit path only. It intentionally skips guest-control
  activation because there may be no running guest.
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars
  were retired; the daemon-only contract is the only path.

**Bash**

- There is no live bash fallback for this verb.
### `test`

**Synopsis:** `d2b test <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`test` is a daemon-native live guest activation verb for temporary
activation until reboot. If neither mutation flag is set, the CLI prints
the parity notice and defaults to `--dry-run`; `--apply` requires the VM
to be running and reachable over guest-control.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Publish the prepared VM closure into the VM's live store pool, ask guestd to run a test activation of that prepared toplevel inside the running guest, poll guest activation status, then commit the successful host-side metadata needed to observe the temporary generation. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `1` | `d2bd` is unreachable or guest-control transport fails before a typed activation result is available. | [`daemon-down`](./error-codes.md#daemon-down), [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Host store publication, broker commit, guest activation capability, or guest activation status failed closed. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

Stopped/offline VMs fail closed for `test --apply`. Use `boot --apply`
for offline staging, or start the VM and retry once guest-control
advertises activation support.

**Human example**

```text
$ d2b test corp-vm --apply
d2b test --apply activated in guest via guest-control (vm=corp-vm, mode=test, summary=activated until reboot, generationNumber=42)
```

**Native**

- `--apply`: routes through `d2bd`, guest-control activation, and a
  broker commit after guestd reports success. There is no host-side
  execution of guest activation scripts.
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars
  were retired; the daemon-only contract is the only path.

**Bash**

- There is no live bash fallback for this verb.
### `rollback`

**Synopsis:** `d2b rollback <vm> [--dry-run | --apply] [--human | --json]`

**Status**

`rollback` is a daemon-native live guest rollback verb. If neither
mutation flag is set, the CLI prints the parity notice and defaults to
`--dry-run`; live `--apply` requires the VM to be running and reachable
over guest-control.

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan the activation without mutating the guest. |
| `--apply` | boolean | `false` | Select the previous prepared VM toplevel, ensure it is published into the VM's live store pool, ask guestd to activate it inside the running guest, poll guest activation status, then commit the successful rollback generation through the broker. |
| `--json` | boolean | `false` | Emit the dry-run summary or typed mutating-verb envelope as JSON. |
| `--human` | boolean | `false` | Force the human summary on stdout. |

**Arguments**

| Argument | Semantics |
| --- | --- |
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

**Exit codes**

| Code | Meaning | Typed error / reference |
| --- | --- | --- |
| `0` | Dry-run plan rendered or `--apply` completed successfully. | — |
| `1` | `d2bd` is unreachable or guest-control transport fails before a typed activation result is available. | [`daemon-down`](./error-codes.md#daemon-down), [`generic`](./error-codes.md#generic) |
| `2` | Unknown flag or unsupported invocation shape. | [`usage`](./error-codes.md#usage) |
| `70` | The named VM is not declared in the active manifest. | [`not-found`](./error-codes.md#not-found) |
| `78` | Host store publication, broker commit, guest activation capability, or guest activation status failed closed. | [`broker-error`](./error-codes.md#broker-error), [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |

Live rollback of a stopped/offline VM fails closed. Offline rollback is
not inferred from host-side metadata, and `boot --apply` stages only the
currently declared toplevel for the next start; rollback itself does not
run guest activation from the host.

**Human example**

```text
$ d2b rollback corp-vm --apply
d2b rollback --apply activated previous toplevel in guest via guest-control (vm=corp-vm, mode=rollback, summary=rolled back, generationNumber=41)
```

**Native**

- `--apply`: routes through `d2bd`, guest-control activation, and a
  broker commit after guestd reports success. There is no host-side
  execution of guest activation scripts.
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars
  were retired; the daemon-only contract is the only path.

**Bash**

- There is no live bash fallback for this verb.


### `generations`

**Synopsis:** `d2b generations <vm>`

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
$ d2b generations corp-vm
=== Host-side per-VM store generations (/var/lib/d2b/vms/corp-vm/store-meta/generations) ===
  (none yet — run 'd2b build corp-vm')

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

**Synopsis:** `d2b gc [--dry-run | --apply] [--human | --json]`

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

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `d2b gc` exit table is preserved in this file as history.

**Human example**

```text
$ d2b gc --apply
d2b gc --apply executed via the native daemon → broker path (retainedStorePaths=12, keepGenerations=None, summary=pruned d2b-managed store roots)
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `d2bd` → broker `RunGc` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `store verify`

**Synopsis:** `d2b store verify <vm> [--repair] [--human | --json]`

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
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

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

- Routes through `d2bd` → broker `StoreVerify`.
- Verifies the current marker/manifest and top-level `live/` basenames
  and writes host-only integrity records.
- `--repair` never claims success from the StoreSync attempt alone; it
  returns `repaired` only after the post-repair verification is clean.

**Human example**

```text
$ d2b store verify corp-vm
store verify corp-vm: status=ok checked=42 drifted=0 repaired=0
```

**Bash**

- There is no bash execution path for this verb.

### `trust`

**Synopsis:** `d2b trust <vm> [--dry-run | --apply] [--human | --json]`

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

The historical bash fallback was retired in v1.0 (ADR 0015); v1.0 daemon-unreachable returns exit-78. The legacy `d2b trust` exit table is preserved in this file as history.

**Human example**

```text
$ d2b trust corp-vm --apply
d2b trust --apply executed via the native daemon → broker path (vm=corp-vm, staticIp=10.20.0.10, knownHostsPath=/var/lib/d2b/known_hosts.d2b, updated=true)
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `d2bd` → broker `RunHostKeyTrust` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `rotate-known-host`

**Synopsis:** `d2b rotate-known-host <vm> [--dry-run | --apply] [--human | --json]`

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
$ d2b rotate-known-host corp-vm --apply
d2b rotate-known-host --apply executed via the native daemon → broker path (vm=corp-vm, staticIp=10.20.0.10, knownHostsPath=/var/lib/d2b/known_hosts.d2b, removed=true)
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `d2bd` → broker `RunRotateKnownHost` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


### `keys list`

**Synopsis:** `d2b keys list [--human] [--json]`

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
| `1` | `d2bd` is unreachable; the typed `#daemon-down` envelope is emitted (the v1.0 daemon-only contract — there is no bash fallback; the v1.0 clean-break per ADR 0015 retired the legacy fallback in v1.0). The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) (Category 2 — daemon-down rendering pointer) points operators at the daemon-startup runbook. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unsupported invocation shape inherited from the `keys` subcommand dispatcher. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ d2b keys list
VM                       ENV          FINGERPRINT                                                      MANAGED KEY
corp-vm                  work         SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA              /var/lib/d2b/keys/corp-vm_ed25519
```

**`--json` example**

```json
{
  "command": "keys list",
  "entries": [
    {
      "vm": "corp-vm",
      "env": "work",
      "managedKeyPath": "/var/lib/d2b/keys/corp-vm_ed25519",
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

**Synopsis:** `d2b keys show <vm> [--human] [--json]`

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
| `1` | `d2bd` is unreachable (typed `#daemon-down` envelope; multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the daemon-startup runbook) — OR the daemon returned the request but the key material was unreadable (typed `#generic` envelope; rare). | [`daemon-down`](./error-codes.md#daemon-down) / [`generic`](./error-codes.md#generic) |
| `2` | Unknown VM, missing VM argument, or unreadable key material reported by daemon as an unknown subject. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ d2b keys show corp-vm
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockedExampleKeyForDocsOnly corp-vm_ed25519.pub
```

**Status**

Keys show is a native preview that reports daemon-resolved key metadata placeholders without falling back to bash.

**Native**

- Native per-VM managed-key lookup over the daemon public socket.

**Bash**

- There is no live bash fallback for this verb; the bash disposition is retained only as coverage taxonomy / the retired path.
### `keys rotate`

**Synopsis:** `d2b keys rotate <vm> [--dry-run | --apply] [--human | --json]`

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
| `vm` | Required VM name as declared in `d2b.vms.<name>`. |

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
$ d2b keys rotate corp-vm --apply
d2b keys rotate --apply executed via the native daemon → broker path (vm=corp-vm, fingerprint=SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, keyPath=/var/lib/d2b/keys/corp-vm_ed25519)
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable surfaces `daemon-down` exit 1; native-handler-deferred surfaces `not-yet-implemented` exit 78; `broker-error` exit 78. The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative: wired through `d2bd` → broker `RunKeysRotate` (commit `7de9194`).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.


### `audit`

**Synopsis:** `d2b audit [--strict] [--human] [--json]`

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
| `78` | **`--strict` flag arm only** — `d2b audit --strict` emits typed `#not-yet-implemented` envelope unconditionally regardless of daemon state per [ADR 0017](../adr/0017-no-bash-fallbacks-invariant.md) § "Migration target table" line 91 (the strict-audit surface is queued for v1.2+ (unscheduled; v1.1 only delivers the typed-envelope rendering + remediation per ADR 0017) implementation). The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the migration runbook. | [`not-yet-implemented`](./error-codes.md#not-yet-implemented) |
| `0` | Success (non-`--strict` arm only — `--strict` returns exit 78 unconditionally per above). | — |
| `1` | (Non-`--strict` arm only) `d2bd` is unreachable; typed `#daemon-down` envelope emitted. The multi-line `Remediation:` block per [`error-codes.md` "Remediation rendering conventions"](./error-codes.md#remediation-rendering-conventions) points operators at the daemon-startup runbook. | [`daemon-down`](./error-codes.md#daemon-down) |
| `2` | Unknown flag or unexpected positional argument. | [`usage`](./error-codes.md#usage) |

**Human example**

```text
$ d2b audit --human

=== d2b security audit ===

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

**Synopsis:** `d2b host check [--strict] [--read-only] [--human] [--json]`

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
$ d2b host check
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

**Synopsis:** `d2b host prepare [--dry-run | --apply] [--human | --json]`

**Status**

`host prepare` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "d2b: NOTICE: defaulting to --dry-run; d2b 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`. `--dry-run` is wired live; `--apply` is **not yet wired** — the daemon-side typed-intent dispatch and bundle resolver that back it are still pending, so it returns the typed `daemon-down` envelope (exit 1) today (use `--dry-run` for now). On a Tier-0 legacy/mixed host, `--apply` is refused with `tier-0-legacy-uses-nixos-module` / `single-writer-conflict` (exit 78).

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
$ d2b host prepare --dry-run
host prepare --dry-run: would reconcile nftables + routes + sysctls + /etc/hosts + NetworkManager unmanaged state
```

**Native**

- `--apply`: **not yet wired** — returns the typed `daemon-down` envelope (exit 1) today; re-run with `--dry-run` for now. When the daemon-side dispatch ships, `--apply` will route through `d2bd` → broker; daemon-unreachable will surface `daemon-down` exit 1, native-handler-deferred `not-yet-implemented` exit 78, and `broker-error` exit 78. On a Tier-0 legacy/mixed host `--apply` is refused today (exit 78). The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative (forthcoming): once the public-socket dispatch ships, `--apply` wires `d2bd` → broker `ApplyNftables` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNmUnmanaged` (broker ops staged in commit `ee6ed0b`; public-socket dispatch pending).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.
### `host destroy`

**Synopsis:** `d2b host destroy [--dry-run | --apply] [--human | --json]`

**Status**

`host destroy` is a daemon-native host-reconcile verb. If neither mutation flag is set, stderr emits "d2b: NOTICE: defaulting to --dry-run; d2b 1.0 will require explicit --dry-run or --apply" and the CLI defaults to `--dry-run`. `--dry-run` is wired live; `--apply` is **not yet wired** — the daemon-side typed-intent dispatch and bundle resolver that back it are still pending, so it returns the typed `daemon-down` envelope (exit 1) today (use `--dry-run` for now). On a Tier-0 legacy host, `--apply` is refused with `tier-0-legacy-uses-nixos-module` (exit 78).

**Flags**

| Flag | Type | Default | Semantics |
| --- | --- | --- | --- |
| `--dry-run` | boolean | implicit if neither mutation flag is set | Plan removal of d2b-owned host reconcile state without mutating host state. |
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
$ d2b host destroy --dry-run
host destroy --dry-run: no d2b-owned resources to remove
```

**Native**

- `--apply`: **not yet wired** — returns the typed `daemon-down` envelope (exit 1) today; re-run with `--dry-run` for now. When the daemon-side dispatch ships, `--apply` will route through `d2bd` → broker; daemon-unreachable will surface `daemon-down` exit 1, native-handler-deferred `not-yet-implemented` exit 78, and `broker-error` exit 78. On a Tier-0 legacy host `--apply` is refused today (exit 78). The historical bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Broker failures surface on stderr with the redacted public-safe remediation from security fix `4dde2b9` and exit `78`.
- LiveNative (forthcoming): once the public-socket dispatch ships, `--apply` wires the same broker-op set in reverse order: `ApplyNmUnmanaged` + `ApplyRoute` + `ApplySysctl` + `UpdateHostsFile` + `ApplyNftables` (broker ops staged in commit `ee6ed0b`; reverse-order hardening in `b73e28f`; public-socket dispatch pending).

**Bash**

- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.

### `host migrate-storage`

**Synopsis:** `d2b host migrate-storage [--dry-run | --apply | --rollback --from-checkpoint <id>] [--human | --json]`

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
  "rollbackCommand": "d2b host migrate-storage --rollback --from-checkpoint storage-cutover-…",
  "vmCount": 2,
  "vms": ["corp-vm", "work-vm"],
  "preflightRequirements": [
    "all d2b VMs stopped",
    "d2bd.service stopped",
    "d2b-priv-broker.service stopped",
    "net VMs stopped; guest routing, TAP connectivity, and dependent bridge traffic will be interrupted"
  ],
  "preserve": [
    "per-VM swtpm NVRAM and swtpm identity markers",
    "declared host bridges, TAP naming intent, nftables/NM/networkd ownership metadata, and network-preflight evidence"
  ],
  "cutoverOnlyCleanup": [
    "/run/d2b-gpu",
    "boot-scoped runtime socket files only after all d2b services are stopped"
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
$ d2b host migrate-storage --dry-run
host migrate-storage --dry-run: checkpoint=storage-cutover-… vm_count=2
rollback command: d2b host migrate-storage --rollback --from-checkpoint storage-cutover-…
preflight requirements:
  - all d2b VMs stopped
  - d2bd.service stopped
  - d2b-priv-broker.service stopped
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

**Synopsis:** `d2b host reconcile-otel-acls [--dry-run | --apply] [--human | --json]`

Reconciles per-VM OTel relay/bridge filesystem ACLs to the
current bundle's intended state. Replaces the v0.4-era
`host-otel-relay-acl.nix` activation script (retired in v1.1
per [ADR 0018](../adr/0018-microvm-nix-removal.md) "Host-OTel
ACL migration table"). Invoked from
`system.activationScripts.d2bReconcileOtelAcls` on every
`nixos-rebuild switch` and from `d2bd.service` `ExecStartPost=`
on daemon startup; operators may also invoke it directly for
mid-cycle reconciliation.

**Behaviour**

- `--dry-run` (default if neither flag given): reports the
  planned ACL set/revoke ops without dispatching to the broker;
  exit 0 with a planned-ops summary.
- `--apply`: dispatches through `d2bd` →
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

**Synopsis:** `d2b host doctor --read-only [--human] [--json]`

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
| `D2B_BROKER_SOCKET` | `/run/d2b/broker.sock` | Probe target for `broker-ready`. |
| `D2B_PUBLIC_SOCKET` | `/run/d2b/public.sock` | Probe target for `daemon-ready`. |
| `D2B_DAEMON_STATE_DIR` | `/var/lib/d2b/daemon-state` | Where the daemon writes pidfd/module/autostart reports. |
| `D2B_METRICS_URL` | `http://127.0.0.1:9101/metrics` | URL probed by `metrics-endpoint`. |

**Exit codes**

| Exit | Meaning | Catalog anchor |
| --- | --- | --- |
| `0` | Every check passed. | — |
| `1` | At least one check is `warn`, none are `fail`. | [`host-check-warning`](./error-codes.md#host-check-warning) |
| `2` | At least one check is `fail` (e.g. required kernel module missing, autostart VM failed, broker socket unreachable). | [`host-check-warning`](./error-codes.md#host-check-warning) |
| `78` | Missing required `--read-only` flag (doctor is read-only; mutation forms are later deliverables). | [`--read-only-required`](./error-codes.md#--read-only-required) |

**Disposition:** `rust-native`.

### `host install`

**Synopsis:** `d2b host install (--dry-run | --apply [--enable] [--start | --no-start]) [--human | --json]`

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
| `--enable` | boolean | `false` | After `--apply`, enable `d2bd.service`. |
| `--start` | boolean | `false` | After `--apply --enable`, start `d2bd.service`. |
| `--no-start` | boolean | `false` | After `--apply`, leave `d2bd.service` stopped. |
| `--json` | boolean | `false` | Emit the stable JSON plan or typed error envelope. |
| `--human` | boolean | `false` | Force the human summary. |

**Exit codes**

| Exit | Meaning | Catalog anchor |
| --- | --- | --- |
| `0` | Dry-run plan rendered or daemon → broker apply succeeded. | — |
| `78` | Missing `--dry-run` / `--apply`, or the daemon → broker apply path returned `broker-error`. | [`--apply-or-dry-run-required`](./error-codes.md#--apply-or-dry-run-required), [`broker-error`](./error-codes.md#broker-error) |

**Disposition:** `rust-native` (`--apply` dispatches through daemon → broker `RunHostInstall`).

### `migrate`

**Synopsis:** `d2b migrate (--dry-run | --apply) [--human | --json]`

`migrate` is the migration analyzer. `--dry-run` reports the current
deployment-shape tier plus the stable migration checklist. Per-VM
supervisor classification is still unavailable on the public manifest, so
the planner keeps that limitation explicit and points operators at
`d2b status <vm>` for per-VM truth. `--apply` uses the daemon-first
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
$ d2b migrate --dry-run
d2b migrate --dry-run: deployment shape = tier-0-mixed, 2 VM(s) in manifest.
Per-VM supervisor classification is not available on the public manifest today.
Use `d2b status <vm>` to inspect each VM directly; `d2b migrate --apply`
is the live mutation path when you are ready.
```

**Native**

- `--apply`: routes through `d2bd` → broker. Daemon-unreachable
  surfaces `daemon-down` exit 1; native-handler-deferred surfaces
  [`not-yet-implemented`](./error-codes.md#not-yet-implemented) exit 78;
  [`broker-error`](./error-codes.md#broker-error) exit 78. The historical
  bash fallback was retired in v1.0 (per ADR 0015).
- The `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1` env vars were retired in v1.0; in v1.0 (ADR 0015) the daemon-only contract is the only path. Native refusals stay on the typed envelope, and broker failures surface with exit `78`.
- Dry-run analysis is pure Rust; `--apply` dispatches through `d2bd` →
  broker `RunMigrate`.

**Bash**

  `d2b migrate` path directly.
- In v1.0 daemon-only, `exec_legacy_passthrough` always returns the typed `not-yet-implemented` envelope (exit 78 per ADR 0015); the historical bash-fallback shim was retired in v1.0.

**Disposition:** `rust-native` — dry-run analysis is native, and
`--apply` uses daemon → broker `RunMigrate` when available.

### `auth status`

**Synopsis:** `d2b auth status [--human] [--json]`

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
$ d2b auth status
role: launcher
public socket: /run/d2b/public.sock (reachable, server=0.2.0, selected=0.2.0)
allowed commands: auth status, host check, list, status
denied commands:
- audit: requires admin role in d2b.site.adminUsers
```

**`--json` example** — schema: [`auth-status.schema.json`](./cli-output/auth-status.schema.json); prose companion: [`auth-status.md`](./cli-output/auth-status.md).

```json
{
  "role": "launcher",
  "publicSocket": {
    "path": "/run/d2b/public.sock",
    "reachable": true,
    "serverVersion": "0.2.0",
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
      "reason": "requires admin role in d2b.site.adminUsers"
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

**Synopsis:** `d2b config sync <vm> [--guest-path <path>] [--host <h>] [--user <u>] [--key <path>] [--known-hosts <path>] [--dry-run] [--json]`

<a id="config-sync-guest-control-transport"></a>
On a guest-control-capable VM, `config sync` reads the VM's canonical
guest config working copy (default `/var/lib/d2b-guest/guest-config.nix`)
over the authenticated **guest-control transport** — a typed
`readGuestConfig` request to `d2bd` over the daemon public socket.
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
(`${XDG_STATE_HOME:-~/.local/state}/d2b/config-staging/<vm>.guest.nix`).
The staging copy is never evaluated until approved.

`--dry-run` selects and reports the transport WITHOUT contacting the
daemon or reading any guest bytes: it emits `transport: "guest-control"`
plus the planned staging target only — never an SSH argv and never guest
content.

Fail-closed behaviour:

- `config sync` is **admin-only**: `readGuestConfig` is gated to the
  `d2b-admin` role (`d2b.site.adminUsers`) at the daemon's
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

`d2b host shutdown-hook --apply` is intentionally absent from the public
clap/completion surface and is invoked by `d2bd.service` only while the host
manager is stopping. When that hook connects as uid `0`, the daemon assigns the
narrow `HostShutdown` role. This role can dispatch only `vmStop`; it cannot run
admin-only operator verbs such as exec, USB attach/detach, host prepare/destroy,
audit export, key rotation, or config sync.

**Disposition:** `rust-native` — host-initiated typed `readGuestConfig`
over the daemon public socket; no SSH, no virtiofs, no new privileged
surface.

### `config diff`

**Synopsis:** `d2b config diff <vm> --against <guestConfigFile> [--json]`

Shows a unified diff between the staged guest config and the live
host-side file the operator names with `--against` (typically their
`guestConfigFile`). Exits 0 whether or not they differ; `--json`
reports `differs` + the diff text.

**Disposition:** `rust-native` — read-only `diff -u`.

### `config approve`

**Synopsis:** `d2b config approve <vm> --to <target-file> [--json]`

Validates the staged guest config (non-empty, valid UTF-8) and
atomically writes it onto the operator-chosen `--to` target (unique
`O_EXCL` temp + fsync + rename + parent-dir fsync), then clears the
staging file. The CLI never auto-locates the operator's config tree —
the operator names the target explicitly. The authoritative containment
+ eval gate is the per-VM `guestConfigFile` assertion that runs on the
subsequent `d2b switch`.

**Disposition:** `rust-native` — host-operator-only; atomic publish.

### `config reject`

**Synopsis:** `d2b config reject <vm> [--json]`

Discards the staged guest config for a VM.

**Disposition:** `rust-native`.

### `config status`

**Synopsis:** `d2b config status [<vm>] [--all] [--json]`

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
| `75` | `config sync` only. The caller is not in `d2b.site.adminUsers`. `config sync` dispatches the admin-only `ReadGuestConfig` daemon verb, so a launcher-role peer is rejected with the typed `authz-not-admin` (AUTH) error — exit `75`, the daemon's reserved authz code — before any guest read. The staging-only verbs (`diff`/`approve`/`reject`/`status`) dispatch no daemon verb and so never return `75`. |

With `--json` each verb emits a single stdout object:

- `config sync` → `{ "command": "config sync", "vm", "transport": "guest-control", "staging", "bytes", "sha256" }`
  (or `{ "command": "config sync", "mode": "dry-run", "vm", "transport": "guest-control", "staging", "guestFile" }` under `--dry-run` — no SSH argv, no guest bytes).
- `config diff` → `{ "command": "config diff", "vm", "against", "staging", "differs": <bool>, "diff": <string> }`.
- `config approve` → `{ "command": "config approve", "vm", "target", "bytes" }`.
- `config reject` → `{ "command": "config reject", "vm", "removed": <bool> }`.
- `config status` → `{ "command": "config status", "pending": [ <vm>… ] }`
  (the single-VM form reports a list with 0 or 1 entry).

Pending-staging notes (`d2b status`, `d2b vm start`, and the mutating
verbs) are emitted on **stderr** for human output only, so they never perturb a
`--json` stdout envelope.

### `shell`

**Synopsis:** `d2b shell <target> [ACTION] [--name NAME] [--force] [--json|--human]`

`ACTION` is one of:

- omitted or `attach` — attach to the target's configured default shell session,
  or to `--name NAME`;
- `list` — list persistent shell sessions;
- `detach` — detach a live/stale client without killing the shell;
- `kill` — terminate a named shell session.

The first positional after `shell` is always a d2b target address. Declared
local VM names retain their existing behavior. Canonical direct-local workload
targets and unambiguous workload-id aliases resolve inside `d2bd`: transition
local VMs use `legacyVmName`, first-class local VMs use the workload id, and
unsafe-local targets stay canonical rather than being coerced to VM names.
A local VM named `list`, `attach`, `detach`, or `kill` attaches by default; use
`d2b shell <target> <ACTION>` for management. Command-like trailing words such as
`d2b shell work htop` are rejected with a hint to use
`d2b vm exec <target> -- <cmd>` for one-off commands.

`shell` keeps declared local VM names on the local daemon public socket and the
authenticated guest-control terminal transport. Unsafe-local targets use the
same public `ShellOp` shape, but d2bd resolves bundle-owned policy and the exact
requester-UID helper, then multiplexes the validated helper terminal fd behind
an opaque public attachment handle. List/detach/kill use helper management
operations. Disconnect and `closeAttach` detach only; kill tears down only the
verified shell scope. Gateway-backed management forms
(`list`, `detach`, `kill`) resolve the local realm entrypoint, verify the gateway
VM is running, and run the same `d2b shell <target> ...` command inside the
gateway VM over the typed `vm exec` guest-control path. The host does not load
realm credentials, provider transports, raw guest-control frames, SSH, or
provider-native shell APIs.

All shell actions remain admin-only. Launcher authorization for configured exec
items does not extend to shell. Unsafe-local policy (`defaultName` and
`maxSessions`) never appears in the public request and cannot be supplied by a
client.

Interactive gateway `attach` is fail-closed in this generation with an
actionable `gateway-shell-attach-unavailable` error. Use
`d2b realm enter <realm>` and run `d2b shell <target>` inside the
gateway until semantic ADR 0039 shell attach is implemented. [ADR
0039](../adr/0039-constellation-persistent-shell-routing.md) defines the final
constellation route: gateway-backed targets forward through the selected gateway
and require the remote node or provider agent to advertise `persistent-shell`.

**Flags**

| Flag | Applies to | Semantics |
| --- | --- | --- |
| `--name NAME` | attach, detach, kill | Persistent shell session name. Omitted attach/detach uses the configured default; kill requires `--name`. |
| `--force` | attach | Detach an already-attached client for the same named session before attaching. |
| `--json` | list, detach, kill | Emit one JSON document on stdout. Attach is human/TTY-only and rejects JSON. |
| `--human` | list, detach, kill | Force human output. Attach is always human/TTY-only. |

**Shell name rule**

Names are 1-64 ASCII bytes, start with `[A-Za-z0-9_]`, and then contain only
`[A-Za-z0-9._-]`. Names are user-visible operational identifiers, but daemon
metrics never use names or terminal handles as labels.

**Human examples**

```text
$ d2b shell work
attached to shell 'default' on vm 'work'; detach with Ctrl-Space Ctrl-q; exit or Ctrl-D ends the session
```

```text
$ d2b shell tools.host.d2b
attached to shell 'primary' on vm 'tools.host.d2b'; detach with Ctrl-Space Ctrl-q; exit or Ctrl-D ends the session
```

```text
$ d2b shell work list
NAME    STATE     ATTACHED  DEFAULT
default detached  false     true
```

**`--json` examples**

```json
{
  "command": "shell list",
  "vm": "work",
  "default_name": "default",
  "sessions": [
    {
      "name": "default",
      "state": "detached",
      "attached": false,
      "is_default": true
    }
  ]
}
```

```json
{
  "command": "shell detach",
  "vm": "work",
  "name": "default",
  "result": "already-detached-or-absent",
  "cause": null
}
```

```json
{
  "command": "shell kill",
  "vm": "work",
  "name": "build",
  "result": "killed",
  "state": "killed"
}
```

The JSON field remains named `vm` for the current schema. For local VM targets
it contains the resolved backing VM name; for unsafe-local it carries the
configured canonical workload target. Gateway-backed management commands
forward the requested target through the selected gateway; the in-gateway
response keeps its own current schema until a future output-version bump can
rename this field to `target`.

**Exit codes**

| Code | Meaning |
| --- | --- |
| `0` | Success, including idempotent detach/kill no-op results. |
| `1` | Unexpected daemon reply or local protocol/serialization failure. |
| `2` | Usage error, invalid flag combination, missing required `--name` for kill, invalid shell name, non-TTY attach, or gateway-backed interactive attach before semantic shell attach support lands. |
| `42` | Internal scope/daemon failure. |
| `69` | Daemon/helper/user-manager/terminal transport unavailable or timed out. |
| `70` | Required shell capability or `unsafe-local-shell-v1` is unavailable. |
| `75` | Admin authorization failed, another attachment owns the shell, or capacity is exhausted. |
| `76` | Protocol, operation-correlation, name, cursor-gap, offset, or terminal-size failure. |
| `77` | Stale public attachment handle or authenticated guest session. |

**Redaction**

Shell management JSON may include validated shell names because they are
operator-facing identifiers. Daemon audit records use a fixed shell correlation
digest and may include the configured canonical target and peer uid; metrics
labels are closed provider/component/operation/outcome/error enums. Neither
surface carries shell names, terminal session handles, helper diagnostics,
supervisor metadata, paths, argv, env, cwd, transcripts, or terminal bytes.

### `vm exec`

**Synopsis:** `d2b vm exec [-i] [-t] [-d|--detach] [--env KEY=VALUE]… [--cwd DIR] [--json|--human] <vm> -- <cmd> [args…]`

**Detached management synopsis:**

- `d2b vm exec [--json] <vm> list`
- `d2b vm exec [--json] <vm> logs <exec-id>`
- `d2b vm exec [--json] <vm> status <exec-id>`
- `d2b vm exec [--json] <vm> kill <exec-id>`

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
| `vm` | Required VM name as declared in `d2b.vms.<name>`. Management words such as `status` and `logs` are valid VM names because command execution always uses `--`. |
| `cmd [args…]` | Guest command and arguments after `--`. `argv[0]` may be a bare command name, absolute path, or relative path; it must be non-empty and must not start with `-`. |
| `list` | Detached management verb: list retained detached exec metadata. |
| `logs <exec-id>` | Detached management verb: emit retained stdout/stderr bytes plus bounded metadata warnings, optionally starting at per-stream offsets. |
| `status <exec-id>` | Detached management verb: print state, terminal disposition, and aggregate retained-log metadata. |
| `kill <exec-id>` | Detached management verb: request cancellation; repeated cancellation of a terminal exec reports `already-terminal`. |

Exec command forms **always require `--`** before `<cmd>`. Tokens after
`<vm>` without `--` are management verbs; an unknown verb is a usage
error that tells the operator to use `--` to run a command. This means
`list`, `logs`, `status`, and `kill` remain valid VM names:
`d2b vm exec list -- bash` runs `bash` in a VM named `list`, while
`d2b vm exec list status <id>` asks that VM for a detached exec's
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
$ d2b vm exec -it corp-vm -- bash
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

- **create** — `d2b vm exec -d <vm> -- <cmd> [args…]`: human output
  is one copy-pasteable `exec_id` line. JSON emits
  `{ "command": "vm exec", "vm": "<vm>", "execId": "<id>", "state": "<state>" }`.
- **list** — `d2b vm exec <vm> list`: human output is a table with
  `execId`, state, start time, terminal status when available, aggregate
  and per-stream retained offset windows, and aggregate/per-stream
  dropped/truncated metadata. JSON
  emits `{ "command": "vm exec list", "vm": "<vm>", "execs": [ { "execId",
  "state", "startedAt", "exitCode"?, "signal"?, "startOffset",
  "endOffset", "droppedBytes", "truncated" } ] }`; implementations also
  expose per-stream stdout/stderr offsets and dropped/truncated flags for
  resume-capable clients.
- **status** — `d2b vm exec <vm> status <exec-id>`: human output is
  the state plus terminal disposition. JSON emits
  `{ "command": "vm exec status", "vm": "<vm>", "execId": "<id>",
  "state", "reason"?, "exitCode"?, "signal"?, "startOffset",
  "endOffset", "droppedBytes", "truncated" }`.
- **logs** — `d2b vm exec <vm> logs <exec-id>`: human output writes
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
- **kill** — `d2b vm exec <vm> kill <exec-id>`: public name for
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
$ d2b vm exec work -- id
uid=1000(alice) gid=100(users)
$ d2b vm exec work list
EXEC ID                  STATE                  STARTED AT                EXIT/SIGNAL    OFFSETS                                    DROPPED/TRUNCATED
exec-1                   exited                 2026-06-15T00:00:00Z      exit=0         all=4..18 stdout=4..8 stderr=9..18         all=5/truncated stdout=2/truncated stderr=3/complete
$ d2b vm exec work logs exec-1 --stdout-offset=4 --stderr-offset=9 --max-len=4096
OUT
ERR
d2b: vm exec logs: retained output incomplete (startOffset=4 endOffset=18 droppedBytes=5 truncated=true stdoutStartOffset=4 stdoutEndOffset=8 stdoutNextOffset=10 stdoutEof=false stdoutDroppedBytes=2 stdoutTruncated=true stderrStartOffset=9 stderrEndOffset=18 stderrNextOffset=21 stderrEof=true stderrDroppedBytes=3 stderrTruncated=false)
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
privileged broker op (attached sessions live in-process in `d2bd`;
detached state lives in guestd's detached registry).

## Dispatch capability table

| Command | Current disposition | Rationale |
| --- | --- | --- |
| `list` | `rust-native` | Pure read-only inventory query; the daemon answers it without mutating host or guest state. |
| `vm start` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output; `--apply` routes through the daemon-backed `SpawnRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm stop` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output, including explicit `--force` / `-f` stop intent; `--apply` routes through the daemon-backed `SignalRunner` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm restart` | `rust-native` | The Rust CLI owns parsing and dry-run DAG output, including explicit `--force` / `-f` stop-phase intent; `--apply` routes through the daemon-backed stop+start sequence. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `vm list` | `rust-native` | Daemon-side runtime inventory from d2bd's public socket; daemon-unavailable returns an explicit empty inventory with remediation text. |
| `status` | `rust-native` | Status is a read-only daemon RPC, including the frozen per-VM JSON shape. |
| `status --check-bridges` | `rust-native` | The bridge-health probe is part of the read-only status surface, even though reconcile remains deferred. |
| `usb attach` | `rust-native` | USBIP attach parses and dispatches one intent to `d2bd`; the daemon coordinates broker host bind/firewall/proxy state and authenticated guestd import over guest-control. |
| `usb detach` | `rust-native` | USBIP detach parses and dispatches one intent to `d2bd`; the daemon asks guestd to detach matching imports, then runs broker `UsbipUnbind` / `UsbipProxyReconcile`. |
| `usb probe` | `rust-native` | USBIP probe is a read-only daemon query backed by the broker's `UsbipProxyReconcile` validation pass. |
| `console` | `rust-native` | The Rust CLI owns help / argument validation and attaches to the daemon-native foreground console handoff via `ConsoleOp`, with provider-capability-aware streaming across Cloud Hypervisor, qemu-media, and ACA targets; see [provider capability matrix](./provider-capability-matrix.md). |
| `audio status` | `rust-native` | The Rust CLI dispatches `AudioOp::Status` and renders provider-capability-aware per-target audio state/errors across Cloud Hypervisor, qemu-media, and ACA; see [provider capability matrix](./provider-capability-matrix.md). |
| `audio mic` | `rust-native` | The Rust CLI dispatches microphone grant/revoke through `AudioOp::Mute`, persisting policy and applying host/guest enforcement according to provider capability. |
| `audio speaker` | `rust-native` | The Rust CLI dispatches speaker grant/revoke or level changes through `AudioOp`, persisting policy and applying host/guest enforcement according to provider capability. |
| `audio off` | `rust-native` | The Rust CLI dispatches the `off` shorthand as audio mute operations for both directions, sealing supported host boundaries and reporting any degraded guest/provider enforcement. |
| `build` | `rust-native` | Build is a native non-destructive planner that renders the eval/build preview without falling back to bash. |
| `switch` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `boot` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `test` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `rollback` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunActivation` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `generations` | `rust-native` | Generations is a native introspection surface that reports current/booted symlink targets without falling back to bash. |
| `gc` | `rust-native` | The Rust CLI owns dry-run output; `--apply` routes through the daemon-backed `RunGc` path. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `store verify` | `rust-native` | Routes through `d2bd` → broker `StoreVerify`; the CLI never reads the store-view directly. |
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
| `migrate` | `rust-native` | Dry-run analysis is native; `--apply` routes through `d2bd` → broker `RunMigrate`. Daemon-unreachable / native-handler-deferred conditions surface typed envelopes (exit `1` / exit `78` per ADR 0015); the historical bash fallback was retired in v1.0. |
| `auth status` | `rust-native` | Auth status is a read-only daemon query that reports caller mapping, socket reachability, and authorization hints. |
| `vm exec` | `rust-native` | Daemon public socket → authenticated guest-control session → `guestd` exec RPCs. Admin-only; no SSH, no host PTY, no new privileged broker op. Attached exec uses the in-process `d2bd` session table; detached exec uses guestd's detached registry and VM-first management verbs. |
| `shell` | `rust-native` | Admin-only provider-neutral `ShellOp`: local VMs use authenticated guest-control; unsafe-local uses the exact requester-UID helper and a multiplexed terminal fd. No SSH, host-shell fallback, root unit, per-VM service, or broker op. |
