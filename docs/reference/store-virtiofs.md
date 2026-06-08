# Store + virtiofs share reference (W4)

This reference documents the per-VM virtiofs share set the W4
headless alpha daemon supervises. The shape is anchored by the W0b
[runner-shape audit](runner-shape-audit.md); the W4-H2
[`virtiofsd_argv`](../../packages/nixling-host/src/virtiofsd_argv.rs)
generator emits matching argv.

## The four W4 alpha shares

For the audited headless `corp-vm`:

| Tag           | Socket                                   | Shared dir                                            | Mode |
|---------------|------------------------------------------|-------------------------------------------------------|------|
| `ro-store`    | `corp-vm-virtiofs-ro-store.sock`         | `/nix/store`                                          | RO   |
| `nl-meta`     | `corp-vm-virtiofs-nl-meta.sock`          | `/var/lib/nixling/vms/corp-vm/store-meta`             | RW   |
| `nl-hkeys`    | `corp-vm-virtiofs-nl-hkeys.sock`         | `/var/lib/nixling/vms/corp-vm/host-keys`              | RW   |
| `nl-ssh-host` | `corp-vm-virtiofs-nl-ssh-host.sock`      | `/var/lib/nixling/vms/corp-vm/sshd-host-keys`         | RW   |

CH connects to each socket via the `--fs socket=<path>,tag=<tag>`
flag (see `ChArgvInput.fs_shares` in
[`ch_argv`](../../packages/nixling-host/src/ch_argv.rs)).

## virtiofsd argv shape

Each share renders to one virtiofsd process whose argv matches the
W0b audit:

```text
virtiofsd \
  --socket-path=<vm>-virtiofs-<tag>.sock \
  --socket-group=kvm \
  --shared-dir=<host-path> \
  --thread-pool-size=<N> \
  --posix-acl \
  --xattr \
  --cache=auto \
  --inode-file-handles=prefer \
  [--readonly]
```

Flag semantics:

- `--socket-path` — UDS the CH runner connects to. Daemon-owned;
  the W4 broker places it under `/run/nixling/vms/<vm>/`. The
  audit uses runner-cwd-relative paths; either shape is honoured
  by the argv generator.
- `--socket-group=kvm` — UDS group ownership. The W4 daemon-owned
  broker may move this to a dedicated `nixling-virtiofs` group as
  part of the ADR-0003 minijail split; the generator accepts any
  group string.
- `--shared-dir` — host path the guest sees through the tag.
- `--thread-pool-size` — integer. The daemon caller resolves
  `nproc` at spawn time.
- `--posix-acl`, `--xattr` — both on by default to match the audit
  shape (matters for the `ro-store` share so the guest sees the
  same xattrs the host store has).
- `--cache=auto` — auto-cache (kernel decides per inode). `always`
  is unsafe for the `ro-store` share because hardlink farm churn
  could expose stale store-paths; `never` makes virtiofs latency
  visible. `auto` matches the W0b audit.
- `--inode-file-handles=prefer` — virtiofsd uses `name_to_handle_at`
  when the underlying filesystem supports it. Reduces the per-share
  fd budget; matches the audit shape.
- `--readonly` — only the `ro-store` share has this in the W4 alpha
  shape. The other three shares are RW.

## Daemon-owned uid/gid

Per ADR 0003 each virtiofsd instance runs under a per-role
`nixling-virtiofs` uid/gid the broker provisions at host-prepare
time. The CH runner's `--fs socket=<path>` line trusts the broker
to have set the socket's group ownership to `kvm` (or the migrated
`nixling-virtiofs` group post-ADR-0003).

The daemon never names the uid/gid on the wire (per W3fu1 H1
security-1); the broker resolves the per-role uid from the trusted
bundle when it serves the W4-H5 `SpawnRunner` request.

## Cross-references

- [`nixling_host::virtiofsd_argv`](../../packages/nixling-host/src/virtiofsd_argv.rs)
  — the pure argv generator + 19 unit tests.
- [Runner-shape audit (W0b)](runner-shape-audit.md) — the parity
  oracle for the share set + virtiofsd flags.
- [ADR 0003](../adr/0003-minijail-provisioning-and-sandbox-interface.md)
  — per-role minijail uid/cap split.
- [ADR 0004](../adr/0004-cloud-hypervisor-runner-shape.md) — CH
  runner-shape decision including the virtiofs share contract.
- [Daemon lifecycle](../explanation/daemon-lifecycle.md) — where
  virtiofsd sits in the per-VM DAG.
