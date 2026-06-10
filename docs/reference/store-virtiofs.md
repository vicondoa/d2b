# Store + virtiofs share reference

This reference documents the current daemon-owned per-VM virtiofs share
set. Historical microvm.nix runner evidence lives in
[runner-shape audit](runner-shape-audit.md); current argv comes from
`nixos-modules/processes-json.nix`.

## Framework-managed shares

For a headless `corp-vm`, nixling emits these baseline shares. The
guest-control token share is present only when
`nixling.vms.corp-vm.guest.control.enable = true`.

| Tag           | Socket                                   | Shared dir                                            | Mode |
|---------------|------------------------------------------|-------------------------------------------------------|------|
| `ro-store`    | `/run/nixling/vms/corp-vm/ro-store.sock` | per-VM hardlink farm served as guest `/nix/store`     | RO   |
| `nl-meta`     | `/run/nixling/vms/corp-vm/nl-meta.sock`  | `/var/lib/nixling/vms/corp-vm/store-meta`             | RW   |
| `nl-hkeys`    | `/run/nixling/vms/corp-vm/nl-hkeys.sock` | `/var/lib/nixling/vms/corp-vm/host-keys`              | RW   |
| `nl-ssh-host` | `/run/nixling/vms/corp-vm/nl-ssh-host.sock` | `/var/lib/nixling/vms/corp-vm/sshd-host-keys`      | RW   |
| `nl-gctl`     | `/run/nixling/vms/corp-vm/guest-control/nl-gctl.sock` | `/var/lib/nixling/guest-control-corp-vm` | RO |

CH connects to each socket via the `--fs socket=<path>,tag=<tag>`
flag (see `ChArgvInput.fs_shares` in
[`ch_argv`](../../packages/nixling-host/src/ch_argv.rs)).

## virtiofsd argv shape

Each share renders to one virtiofsd process:

```text
virtiofsd \
  --socket-path=/run/nixling/vms/<vm>/<tag>.sock \
  --socket-group=kvm \
  --shared-dir=<host-path> \
  --thread-pool-size=<N> \
  --sandbox=chroot \
  --inode-file-handles=never \
  --cache=auto \
  [--readonly]
```

Flag semantics:

- `--socket-path` — UDS the CH runner connects to. Daemon-owned;
  the broker places normal share sockets under
  `/run/nixling/vms/<vm>/<tag>.sock`; `nl-gctl` uses the isolated
  `/run/nixling/vms/<vm>/guest-control/nl-gctl.sock` path.
- `--socket-group=kvm` — UDS group ownership. The daemon-owned
  broker may move this to a dedicated `nixling-virtiofs` group as
  part of the ADR-0003 minijail split; the generator accepts any
  group string.
- `--shared-dir` — host path the guest sees through the tag.
- `--thread-pool-size` — integer. The daemon caller resolves
  `nproc` at spawn time.
- `--sandbox=chroot`, `--inode-file-handles=never` — ADR 0021
  broker-pre-established user namespace shape. Reintroducing
  `--sandbox=namespace` or file handles requires a new ADR/update.
- `--cache=auto` — auto-cache (kernel decides per inode). `always`
  is unsafe for the `ro-store` share because hardlink farm churn
  could expose stale store-paths; `never` makes virtiofs latency
  visible.
- `--readonly` — emitted for every share whose schema marks it
  `readOnly`, including `ro-store` and the guest-control token share
  (`nl-gctl`). Other framework shares remain RW.

## Daemon-owned uid/gid

Per ADR 0021 each virtiofsd instance runs fake-root inside a
broker-pre-established single-entry user namespace and has zero host
capabilities. Normal VM shares map namespace UID/GID 0 to the
`nixling-<vm>-runner` stable principal. The guest-control token share
(`nl-gctl`) maps to the narrower `nixling-<vm>-gctlfs` stable
principal and receives only the token directory/file ACLs plus its
dedicated runtime socket directory.

The CH runner's `--fs socket=<path>` line trusts the broker to have set
the socket's group ownership/ACLs so Cloud Hypervisor can connect.

The daemon never names the uid/gid on the wire; the broker resolves
the per-role uid from the trusted bundle when it serves the
`SpawnRunner` request.

## Cross-references

- `nixos-modules/processes-json.nix` — current daemon-owned virtiofsd argv
  emitter.
- [Runner-shape audit](runner-shape-audit.md) — historical microvm.nix
  runner evidence, not the current daemon parity oracle.
- [ADR 0003](../adr/0003-minijail-provisioning-and-sandbox-interface.md)
  — per-role minijail uid/cap split.
- [ADR 0021](../adr/0021-broker-user-namespace-for-virtiofsd.md)
  — broker-pre-established user namespace model for virtiofsd.
- [ADR 0004](../adr/0004-cloud-hypervisor-runner-shape.md) — CH
  runner-shape decision including the virtiofs share contract.
- [Daemon lifecycle](../explanation/daemon-lifecycle.md) — where
  virtiofsd sits in the per-VM DAG.
