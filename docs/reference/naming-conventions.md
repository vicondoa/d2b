# Naming conventions

Canonical reference for host-visible d2b naming. AGENTS.md and the design doc keep shorter summaries; this page is the detailed glossary.

## Service and user naming

| Resource | Pattern | Notes |
| --- | --- | --- |
| Daemon | `d2bd.service` | v1.0 non-root daemon; the only persistent user-facing system unit d2b declares besides the broker (per ADR 0015). |
| Privileged broker | `d2b-priv-broker.{socket,service}` | v1.0 socket-activated privileged dispatcher for every audited host mutation. |
| Per-VM runner leaves | `d2b.slice/<vm>/<role>` | v1.0 broker-spawned runner cgroup leaves (replaces legacy `d2b-<vm>-{gpu,video,snd,swtpm,store-sync}.service`). |
| Per-env runner leaves | `d2b.slice/sys-<env>/usbipd-{backend,proxy}` | v1.0 broker-spawned per-env USBIP runners (replaces legacy `d2b-sys-<env>-usbipd-{backend,proxy}.service`). |
| Legacy per-VM lifecycle wrapper | `d2b@<vm>.service` | Retired in v1.0 (per ADR 0015); v1.0 lifecycle dispatches through `d2bd` → broker `SpawnRunner` instead. |
| Upstream backend | `microvm@<vm>.service` | microvm.nix's own template; still emitted in v1.0 for direct-debug bypass but not the lifecycle-of-record. |
| Legacy per-VM sidecars | `d2b-<vm>-<purpose>.service` | Legacy sidecar systemd templates (`gpu`, `video`, `snd`, `swtpm`, `store-sync`). Retired in v1.0; the broker now spawns the corresponding runners on the per-VM DAG. |
| Virtiofsd exception | `microvm-virtiofsd@<vm>.service` | Upstream microvm.nix unit; rename to `d2b-<vm>-virtiofsd` is tracked for a future release. |
| Legacy per-env system services | `d2b-sys-<env>-<purpose>.service` | Example: `d2b-sys-corp-usbipd-proxy.service`. Retired in v1.0 (per ADR 0015); in v1.0 the broker spawns the equivalent runners under `d2b.slice/sys-<env>/<role>`. |
| Legacy host singleton services | `d2b-<role>.service` | Examples: `d2b-ch-exporter.service`, `d2b-otel-host-bridge.service`. All retired by v1.0; ADR 0015 § "What gets removed" lists the full retired inventory. |
| Auto-declared net VM | `d2b@sys-<env>-net.service` | Framework-reserved net VM per env (legacy wrapper-unit name; in v1.0 dispatched through `d2bd` → broker like every other VM). |
| Legacy per-VM system users | `d2b-<vm>-{gpu,video,snd,swtpm,store-sync}` | Legacy framework-managed per-VM service users; in v1.0 (per [ADR 0015](../adr/0015-daemon-only-clean-break.md)) the same user identities are preserved as the broker-spawned runner uids under `d2b.slice/<vm>/<role>`. Notable exceptions: `d2b-<vm>-store-sync` runs as root (no dedicated user) and `d2b-<vm>-gpu` is shared by the GPU and video runners. |
| Launcher group | `d2b` | v1.2 Unix group allowed to talk to `d2bd` over `/run/d2b/public.sock` (mode 0660, group `d2b`). Authorisation is enforced via SO_PEERCRED at accept time. The pre-v1.2 `d2b-launcher` / `d2b-launchers` groups are empty migration tombstones only. |
| Unsafe-local helper group | `d2b-unsafe-local` | Narrow helper-socket access group populated only from users allowed to use an enabled unsafe-local realm. |
| Unsafe-local helper socket | `/run/d2b/unsafe-local-helper.sock` | Daemon-owned private `SOCK_SEQPACKET` listener; not a systemd socket unit and not a fourth root service. |

## Broker caller-role audit labels

The broker emits `peer_role` / caller-role values into
`/etc/d2b/privileges.json` and every line of
`/var/lib/d2b/audit/broker-<utc-date>.jsonl`.

| Label | Class | Stability |
| --- | --- | --- |
| `d2b-launcher` | lifecycle/launcher principal (members authorized by the daemon to call `public.sock`) | **Stable** — permanent audit-class identifier |
| `d2b-admin` | administrative principal | **Stable** — permanent audit-class identifier |

These are stable broker audit/authz class labels. They are distinct
from the live Unix group name (`d2b` from v1.2 onward; previously
`d2b-launcher` and `d2b-launchers`). Audit consumers must not
cross-reference `peer_role == "d2b-launcher"` with Unix group
membership: the audit label is a class identifier, not a group lookup.

See [ADR 0015](../adr/0015-daemon-only-clean-break.md) for the
daemon-only authz/audit invariants that make these labels stable across
host-side group renames.

## Realm, workload, VM, and env identifiers

- VM name regex: `^[a-z][a-z0-9-]*$`
- Reserved VM prefix: `sys-`
- Reserved VM name: `launcher`
- `sys-<env>-net` is framework-reserved for the auto-declared net VM.

Realm and workload labels use the same lowercase label shape. The canonical
public workload target form is:

```text
<workload>.<realm>[.<ancestor>...].d2b
```

During the v2 transition, `d2b.realms.<realm>.workloads.<workload>.legacyVmName`
maps that public workload id to the existing local VM substrate. For example,
`workloads.aad.legacyVmName = "work-aad"` makes `aad.work.d2b` resolve to the
local `work-aad` VM for status and guest-control exec while preserving
`/var/lib/d2b/vms/work-aad`.

These constraints let the CLI, manifest, and host-side units resolve resources
mechanically without collisions. When docs and code differ, the passing code is
canon; see [AGENTS.md](../../AGENTS.md#existing-code-is-canon).

Launcher item ids also use `^[a-z][a-z0-9-]*$`. They are scoped to one workload
and appear in `d2b launch <target> --item <id>`.

Private configured unsafe-local and local-VM launcher data is installed as
`/etc/d2b/unsafe-local-workloads.json`. Public provider-neutral launcher
metadata uses `/etc/d2b/realm-workloads-launcher-v2.json`; the compatibility
schema remains `/etc/d2b/realm-workloads-launcher.json`.

## Persistent shell session names

Persistent shell names use a separate operational identifier shape:

- 1-64 ASCII bytes.
- First byte: `[A-Za-z0-9_]`.
- Remaining bytes: `[A-Za-z0-9._-]`.
- No whitespace, slash, shell template braces, or leading `-`.

The configured default is `default`. Session names may appear in CLI output and
operator commands, but daemon metrics never use them as labels. Daemon audit
records use a fixed-length digest for shell correlation instead of raw shell
names or terminal session handles.

## Realm target and model identifiers

Realm targets extend the VM/env naming rules without making a target address a
network address. The canonical realm target form is:

```text
<workload>.<realm>[.<ancestor>...].d2b
```

Bare workload names are convenience aliases only when a caller supplies a
default realm or explicit alias table. Fully qualified public targets must
end in `.d2b`; `all`, `*`, and non-suffix `d2b` labels are list selectors or
reserved words, never target labels. Public targets do not include physical
node labels; placement is resolved inside the owning realm.

Label-shaped constellation ids (`RealmId`, `NodeId`, `WorkloadId`,
`ProviderId`) use the same lowercase label shape as VM names:
`^[a-z][a-z0-9-]*$`, bounded to 128 bytes. Opaque ids
(`GatewayId`, `ExecutionId`, `StreamId`, `StreamCursor`, `PrincipalId`,
`OperationId`, and `IdempotencyKey`) are bounded printable-ASCII tokens
without spaces. See
[`realm-core.md`](./realm-core.md) for the complete realm-core model
contract and [Realm access resolver contract](./realm-access-resolver.md) for
resolver diagnostics and access binding behavior.

`d2b.realms.<realm>` uses the same lowercase label shape for the realm
attribute name and default `id`. The realm option schema also exposes a
`path` field for most-specific-first realm paths and an `env` /
`network.envs` bridge to existing `d2b.envs` names. That bridge is
transition metadata only in the current implementation; bridge and TAP
names below are still generated from `d2b.envs` and `d2b.vms.<vm>.env`.
See [Realm option schema](./realm-options.md).

Consumers should read canonical realm and provider identities from generated
metadata rather than reconstructing process or endpoint names from realm paths.

## Network device names

| Resource | Pattern | Notes |
| --- | --- | --- |
| Host↔net-VM bridge | `br-<env>-up` | Host-side uplink bridge for an env. |
| Net-VM↔workload bridge | `br-<env>-lan` | Workload LAN bridge for an env. |
| Net-VM uplink tap | `<env>-u2` | Tap used by the auto-declared net VM uplink. |
| Workload LAN taps | `<env>-l<N>` | Per-workload LAN tap, where `<N>` is the workload index. |

## Examples

- `d2b@work.service`
- `microvm@work.service`
- `d2b-work-gpu.service`
- `d2b-work-video.service`
- `d2b-work-swtpm.service`
- `d2b-work-store-sync.service`
- `microvm-virtiofsd@work.service`
- `d2b-sys-corp-usbipd-proxy.service`
- `d2b-ch-exporter.service`
- `d2b-otel-host-bridge.service`
- `d2b@sys-corp-net.service`
- `br-corp-up`, `br-corp-lan`, `corp-u2`, `corp-l2`

## Host-prepare network IfName conventions

Host prepare introduces a separate name space for the bridges and TAPs
the privileged broker creates on the host on behalf of a daemon-backed
VM (per ADR 0012 — IPv6-off sysctl set, hash-derived IfName,
bridge-port defaults):

- Every broker-created host interface is named
  `d2b-<10-char hash>` for bridges and `d2bv-<10-char hash>` for TAPs.
  The hash is FNV-1a + Crockford base32 over the canonical
  `<env>/<vm>/<role>` tuple from the trusted bundle. The 10-char
  body keeps the full name at ≤ `IFNAMSIZ-1` (15 bytes).
- The mapping is exposed to operators via
  `d2b host check` JSON output and via the
  `IfNameMapping` DTO in `bundle/host.json` (env/vm name → derived
  `d2b-*`/`d2bv-*` name). Operators must never hand-pick an ifname
  whose prefix collides with `d2b-` or `d2bv-`.
- Collisions are detected at emitter time: two bundle tuples that
  hash to the same body fail bundle render with
  `ifname-collision` rather than racing at apply time.
- The legacy per-env bridges (`br-<env>-up`, `br-<env>-lan`,
  `<env>-u2`, `<env>-l2`) belong to the **legacy-systemd** mode
  and are unchanged. They live in a disjoint name space and
  coexist with daemon-mode `d2b-*`/`d2bv-*` names on the same host.

`IFNAMSIZ` enforcement is a hard predicate on every broker-derived
name; an ifname that would exceed 15 bytes is rejected at bundle
render with `ifname-too-long`.

### Deterministic Nix derivation

The Nix emitter (`nixos-modules/host-json.nix`) derives every host
ifname via a single helper, equivalent to:

```nix
# nixos-modules/host-json.nix (sketch)
mkD2bIfName = { kind, env, vm, role }:
  let
    # Canonical "<env>/<vm>/<role>" tuple feeds FNV-1a → Crockford base32.
    key = "${env}/${vm}/${role}";
    hash = lib.substring 0 10
      (lib.fnv1aCrockford32 key);          # 10-char body
    prefix = if kind == "bridge" then "d2b-" else "d2bv-";
    name = prefix + hash;
  in
    assert lib.stringLength name <= 15;     # IFNAMSIZ-1
    name;
```

The same function runs in `packages/d2b-host/src/ifname.rs` so the
broker and the emitter always agree byte-for-byte. The committed
`tests/golden/host-json/ifname-collision.json` fixture pins one
emitter-rejected collision case.

### Looking up the user-visible name from a derived IfName

Operators rarely need to read the `d2b-<hash>` or `d2bv-<hash>` names
directly. Both directions of the mapping ship in the
`IfNameMapping` DTO of `bundle/host.json` and are surfaced to
operators in two CLI outputs:

- `d2b host check --json` — emits the full per-env, per-VM,
  per-role mapping under `.host.ifnameMapping[]`. Use this to look
  up a `d2b-*`/`d2bv-*` name observed in `ip link`, `nft list ruleset`,
  or a broker audit record back to `(env, vm, role)`.
- `d2b status <vm> --json` — emits the same mapping scoped to one
  VM under `.vm.ifnames[]`.

The mapping is keyed on `(kind, env, vm, role)` so the human-visible
columns and the wire-stable `ifname_derived` field always reconcile
unambiguously.

## Related docs

- [AGENTS.md](../../AGENTS.md)
- [Design explanation](../explanation/design.md)
- [Realm option schema](./realm-options.md)
- [Realm core model reference](./realm-core.md)
- [USB/IP component reference](./components-usbip.md)
- [tests/README.md](../../tests/README.md)
- [ADR 0012 — IPv6-off sysctl set, hash-derived IfName, bridge-port defaults](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
- [Host-prepare how-to](../how-to/host-prepare.md)
