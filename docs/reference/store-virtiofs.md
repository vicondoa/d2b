# Store + virtiofs share reference

The Volume Provider exposes the closure-only Guest store view through
broker-owned virtiofs endpoints. Historical microvm.nix runner evidence lives
in [runner-shape audit](runner-shape-audit.md); current launch intent is
derived from the Zone Resource graph.

## Framework-managed shares

For a headless Guest, d2b emits baseline closure and metadata shares.
ComponentSession enrollment keys are never delivered through virtiofs.

| Tag           | Socket                                   | Shared dir                                            | Mode |
|---------------|------------------------------------------|-------------------------------------------------------|------|
| `ro-store`    | broker-resolved private Endpoint | closure-only Guest store view | RO |
| `d2b-meta`    | broker-resolved private Endpoint | Guest-safe generation metadata | RO |
| `d2b-hkeys`   | broker-resolved private Endpoint | Guest activation key projection | RW |
| `d2b-ssh-host` | broker-resolved private Endpoint | Guest host-key projection | RW |

The Cloud Hypervisor Process receives these broker-resolved share descriptors
through its sealed Process Provider launch ticket. The Guest controller does
not construct VMM argv or receive host socket paths; the private
`artifact-catalog.json` Guest closure row carries the complete VMM intent.

## virtiofsd argv shape

Each share renders to one broker-spawned virtiofsd process:

```text
virtiofsd \
  --socket-path=<private-endpoint> \
  [--socket-group=<group>] \
  --shared-dir=<host-path> \
  --thread-pool-size=<N> \
  --sandbox=chroot \
  --inode-file-handles=never \
  --cache=auto \
  [--readonly]
```

Flag semantics:

- `--socket-path` - private Endpoint resolved by the broker. It is never a
  public Resource or CLI argument.
- `--socket-group=<group>` - optional UDS group ownership. It is emitted
  only when `microvm.virtiofsd.group` is non-null.
- `--shared-dir` - anchored host path the Guest sees through the tag; it is
  resolved from the private bundle.
- `--thread-pool-size` - integer resolved from
  `microvm.virtiofsd.threadPoolSize`, falling back to the VM vCPU count
  (or `1` when vCPU is unset/zero).
- `--sandbox=chroot`, `--inode-file-handles=never` - ADR 0021
  broker-pre-established user namespace shape. Reintroducing
  `--sandbox=namespace` or file handles requires a new ADR/update.
- `--cache=auto` - auto-cache (kernel decides per inode). `always`
  is unsafe for the `ro-store` share because hardlink farm churn
  could expose stale store-paths; `never` makes virtiofs latency
  visible. `auto` matches the audit.
- `--inode-file-handles=prefer` - virtiofsd uses `name_to_handle_at`
  when the underlying filesystem supports it. Reduces the per-share
  fd budget; matches the audit shape.
- `--readonly` - `ro-store` and `d2b-meta` are read-only. `d2b-meta` is rooted at
  `store-view/meta` and carries only guest-safe generation metadata
  (`current`, `store-paths`, `db.dump`, allow-listed `meta.json`); it
  never exposes `live/`, `state/`, `gcroots/`, or `sync.lock`. The
  other framework shares remain RW.

## Daemon-owned uid/gid

Each virtiofsd instance runs inside its broker-established user namespace with
zero host capabilities. Namespace identity and uid/gid mapping are private
Provider details, not public Guest names.

The CH runner's `--fs socket=<path>` line trusts the broker to have set
the socket's group ownership/ACLs so Cloud Hypervisor can connect.

The daemon never names the uid/gid or endpoint path on the public wire; the
broker resolves them from the trusted bundle for the `SpawnRunner` request.

## Cross-references

- `nixos-modules/guest-closures.nix` - private evaluated Guest closure and
  Cloud Hypervisor intent emitter.
- [Runner-shape audit](runner-shape-audit.md) - historical microvm.nix
  runner evidence, not the current daemon parity oracle.
- [ADR 0003](../adr/0003-minijail-provisioning-and-sandbox-interface.md) -
  per-role minijail uid/cap split.
- [ADR 0021](../adr/0021-broker-user-namespace-for-virtiofsd.md) -
  broker-pre-established user namespace model for virtiofsd.
- [ADR 0004](../adr/0004-cloud-hypervisor-runner-shape.md) - CH
  runner-shape decision including the virtiofs share contract.
- [Daemon lifecycle](../explanation/daemon-lifecycle.md) - where
  virtiofsd sits in the per-VM DAG.
