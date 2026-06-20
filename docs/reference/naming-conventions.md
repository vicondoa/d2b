# Naming conventions

Canonical reference for host-visible nixling naming. AGENTS.md and the design doc keep shorter summaries; this page is the detailed glossary.

## Service and user naming

| Resource | Pattern | Notes |
| --- | --- | --- |
| Daemon | `nixlingd.service` | v1.0 non-root daemon; the only persistent user-facing system unit nixling declares besides the broker (per ADR 0015). |
| Privileged broker | `nixling-priv-broker.{socket,service}` | v1.0 socket-activated privileged dispatcher for every audited host mutation. |
| Per-VM runner leaves | `nixling.slice/<vm>/<role>` | v1.0 broker-spawned runner cgroup leaves (replaces legacy `nixling-<vm>-{gpu,video,snd,swtpm,store-sync}.service`). |
| Per-env runner leaves | `nixling.slice/sys-<env>/usbipd-{backend,proxy}` | v1.0 broker-spawned per-env USBIP runners (replaces legacy `nixling-sys-<env>-usbipd-{backend,proxy}.service`). |
| Legacy per-VM lifecycle wrapper | `nixling@<vm>.service` | Retired in v1.0 (per ADR 0015); v1.0 lifecycle dispatches through `nixlingd` → broker `SpawnRunner` instead. |
| Upstream backend | `microvm@<vm>.service` | microvm.nix's own template; still emitted in v1.0 for direct-debug bypass but not the lifecycle-of-record. |
| Legacy per-VM sidecars | `nixling-<vm>-<purpose>.service` | Legacy sidecar systemd templates (`gpu`, `video`, `snd`, `swtpm`, `store-sync`). Retired in v1.0; the broker now spawns the corresponding runners on the per-VM DAG. |
| Virtiofsd exception | `microvm-virtiofsd@<vm>.service` | Upstream microvm.nix unit; rename to `nixling-<vm>-virtiofsd` is tracked for a future release. |
| Legacy per-env system services | `nixling-sys-<env>-<purpose>.service` | Example: `nixling-sys-corp-usbipd-proxy.service`. Retired in v1.0 (per ADR 0015); in v1.0 the broker spawns the equivalent runners under `nixling.slice/sys-<env>/<role>`. |
| Legacy host singleton services | `nixling-<role>.service` | Examples: `nixling-ch-exporter.service`, `nixling-otel-host-bridge.service`. All retired by v1.0; ADR 0015 § "What gets removed" lists the full retired inventory. |
| Auto-declared net VM | `nixling@sys-<env>-net.service` | Framework-reserved net VM per env (legacy wrapper-unit name; in v1.0 dispatched through `nixlingd` → broker like every other VM). |
| Legacy per-VM system users | `nixling-<vm>-{gpu,video,snd,swtpm,store-sync}` | Legacy framework-managed per-VM service users; in v1.0 (per [ADR 0015](../adr/0015-daemon-only-clean-break.md)) the same user identities are preserved as the broker-spawned runner uids under `nixling.slice/<vm>/<role>`. Notable exceptions: `nixling-<vm>-store-sync` runs as root (no dedicated user) and `nixling-<vm>-gpu` is shared by the GPU and video runners. |
| Launcher group | `nixling` | v1.2 Unix group allowed to talk to `nixlingd` over `/run/nixling/public.sock` (mode 0660, group `nixling`). Authorisation is enforced via SO_PEERCRED at accept time. The pre-v1.2 `nixling-launcher` / `nixling-launchers` groups are empty migration tombstones only. |

## Broker caller-role audit labels

The broker emits `peer_role` / caller-role values into
`/etc/nixling/privileges.json` and every line of
`/var/lib/nixling/audit/broker-<utc-date>.jsonl`.

| Label | Class | Stability |
| --- | --- | --- |
| `nixling-launcher` | lifecycle/launcher principal (members authorized by the daemon to call `public.sock`) | **Stable** — permanent audit-class identifier |
| `nixling-admin` | administrative principal | **Stable** — permanent audit-class identifier |

These are stable broker audit/authz class labels. They are distinct
from the live Unix group name (`nixling` from v1.2 onward; previously
`nixling-launcher` and `nixling-launchers`). Audit consumers must not
cross-reference `peer_role == "nixling-launcher"` with Unix group
membership: the audit label is a class identifier, not a group lookup.

See [ADR 0015](../adr/0015-daemon-only-clean-break.md) for the
daemon-only authz/audit invariants that make these labels stable across
host-side group renames.

## VM and env identifiers

- VM name regex: `^[a-z][a-z0-9-]*$`
- Reserved VM prefix: `sys-`
- Reserved VM name: `launcher`
- `sys-<env>-net` is framework-reserved for the auto-declared net VM.

These constraints let the CLI, manifest, and host-side units resolve resources mechanically without collisions. When docs and code differ, the passing code is canon; see [AGENTS.md](../../AGENTS.md#existing-code-is-canon).

## Constellation target and model identifiers

Constellation targets extend the VM/env naming rules without making a
target address a network address. The canonical persisted form is:

```text
nl://<workload>.<node>.<realm-path>.nixling
```

The bare `<workload>` form remains the v1-compatible local workload
alias. Multi-label human forms must end in `.nixling`; `all`, `*`, and
non-suffix `nixling` labels are list selectors or reserved words, never
target labels.

Label-shaped constellation ids (`RealmId`, `NodeId`, `WorkloadId`,
`ProviderId`) use the same lowercase label shape as VM names:
`^[a-z][a-z0-9-]*$`, bounded to 128 bytes. Opaque ids
(`GatewayId`, `ExecutionId`, `StreamId`, `StreamCursor`, `PrincipalId`,
`OperationId`, and `IdempotencyKey`) are bounded printable-ASCII tokens
without spaces. See
[`constellation-core.md`](./constellation-core.md) for the complete
core model contract.

## Network device names

| Resource | Pattern | Notes |
| --- | --- | --- |
| Host↔net-VM bridge | `br-<env>-up` | Host-side uplink bridge for an env. |
| Net-VM↔workload bridge | `br-<env>-lan` | Workload LAN bridge for an env. |
| Net-VM uplink tap | `<env>-u2` | Tap used by the auto-declared net VM uplink. |
| Workload LAN taps | `<env>-l<N>` | Per-workload LAN tap, where `<N>` is the workload index. |

## Examples

- `nixling@work.service`
- `microvm@work.service`
- `nixling-work-gpu.service`
- `nixling-work-video.service`
- `nixling-work-snd.service`
- `nixling-work-swtpm.service`
- `nixling-work-store-sync.service`
- `microvm-virtiofsd@work.service`
- `nixling-sys-corp-usbipd-proxy.service`
- `nixling-ch-exporter.service`
- `nixling-otel-host-bridge.service`
- `nixling@sys-corp-net.service`
- `br-corp-up`, `br-corp-lan`, `corp-u2`, `corp-l2`

## Host-prepare network IfName conventions

Host prepare introduces a separate name space for the bridges and TAPs
the privileged broker creates on the host on behalf of a daemon-backed
VM (per ADR 0012 — IPv6-off sysctl set, hash-derived IfName,
bridge-port defaults):

- Every broker-created host interface is named
  `nl-<10-char hash>` for bridges and `nlv-<10-char hash>` for TAPs.
  The hash is FNV-1a + Crockford base32 over the canonical
  `<env>/<vm>/<role>` tuple from the trusted bundle. The 10-char
  body keeps the full name at ≤ `IFNAMSIZ-1` (15 bytes).
- The mapping is exposed to operators via
  `nixling host check` JSON output and via the
  `IfNameMapping` DTO in `bundle/host.json` (env/vm name → derived
  `nl-*`/`nlv-*` name). Operators must never hand-pick an ifname
  whose prefix collides with `nl-` or `nlv-`.
- Collisions are detected at emitter time: two bundle tuples that
  hash to the same body fail bundle render with
  `ifname-collision` rather than racing at apply time.
- The legacy per-env bridges (`br-<env>-up`, `br-<env>-lan`,
  `<env>-u2`, `<env>-l2`) belong to the **legacy-systemd** mode
  and are unchanged. They live in a disjoint name space and
  coexist with daemon-mode `nl-*`/`nlv-*` names on the same host.

`IFNAMSIZ` enforcement is a hard predicate on every broker-derived
name; an ifname that would exceed 15 bytes is rejected at bundle
render with `ifname-too-long`.

### Deterministic Nix derivation

The Nix emitter (`nixos-modules/host-json.nix`) derives every host
ifname via a single helper, equivalent to:

```nix
# nixos-modules/host-json.nix (sketch)
mkNixlingIfName = { kind, env, vm, role }:
  let
    # Canonical "<env>/<vm>/<role>" tuple feeds FNV-1a → Crockford base32.
    key = "${env}/${vm}/${role}";
    hash = lib.substring 0 10
      (lib.fnv1aCrockford32 key);          # 10-char body
    prefix = if kind == "bridge" then "nl-" else "nlv-";
    name = prefix + hash;
  in
    assert lib.stringLength name <= 15;     # IFNAMSIZ-1
    name;
```

The same function runs in `packages/nixling-host/src/ifname.rs` so the
broker and the emitter always agree byte-for-byte. The committed
`tests/golden/host-json/ifname-collision.json` fixture pins one
emitter-rejected collision case.

### Looking up the user-visible name from a derived IfName

Operators rarely need to read the `nl-<hash>` or `nlv-<hash>` names
directly. Both directions of the mapping ship in the
`IfNameMapping` DTO of `bundle/host.json` and are surfaced to
operators in two CLI outputs:

- `nixling host check --json` — emits the full per-env, per-VM,
  per-role mapping under `.host.ifnameMapping[]`. Use this to look
  up a `nl-*`/`nlv-*` name observed in `ip link`, `nft list ruleset`,
  or a broker audit record back to `(env, vm, role)`.
- `nixling status <vm> --json` — emits the same mapping scoped to one
  VM under `.vm.ifnames[]`.

The mapping is keyed on `(kind, env, vm, role)` so the human-visible
columns and the wire-stable `ifname_derived` field always reconcile
unambiguously.

## Related docs

- [AGENTS.md](../../AGENTS.md)
- [Design explanation](../explanation/design.md)
- [Constellation core model reference](./constellation-core.md)
- [USB/IP component reference](./components-usbip.md)
- [tests/README.md](../../tests/README.md)
- [ADR 0012 — IPv6-off sysctl set, hash-derived IfName, bridge-port defaults](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
- [Host-prepare how-to](../how-to/host-prepare.md)
