# `d2b-provider-volume-virtiofs`

`Provider/volume-virtiofs` serves a Volume view to a Host or Guest over
virtiofs. It reconciles `virtiofs.d2bus.org.Export` resources and never
writes a Volume row.

## Identity

| Field | Value |
| --- | --- |
| Provider name | `volume-virtiofs` |
| ResourceTypes | `virtiofs.d2bus.org.Export`; read-only watch of `Volume` |
| Attachment transport | `virtiofs` |
| Worker template | `virtiofsd-worker` |
| Finalizer | `volume-virtiofs/export`, on an Export and nothing else |

## virtiofsd argv

The flag envelope is adapted from the shipped host-side generator, with
three differences the Volume spec freezes:

```
virtiofsd
  --socket-path=<private, adapter-derived>
  --socket-group=<resolved>
  --shared-dir=<resolved from the Volume root descriptor>
  --thread-pool-size=<settings or target vcpu count>
  [--posix-acl]   # only when the attachment asks for it
  [--xattr]       # only when the attachment asks for it
  --cache=<auto|always|never>
  --sandbox=chroot
  --inode-file-handles=never
  [--readonly]    # read-only access, or a view granting no write right
```

`--sandbox` is always `chroot`, `--inode-file-handles` is always `never`,
and there is no free-form extra-argument channel. The renderer is
crate-private: it is the only place a resolved path is joined to the
worker plan, and no public type carries one.

## ADR 0021 invariant

Every worker declares zero host capabilities, does not start as root,
runs a chroot sandbox with a read-only root, and receives its privileges
only inside a user namespace the broker pre-establishes through the
`process-principal-root` mapping class. A declared host capability, a
root start, `--sandbox=namespace`, or a writable root is rejected before
any launch is requested.

## Socket path privacy

The export socket path is generated and private. Only its opaque
`SocketIdentity` is public. The path never appears in a spec field, a
status field, an audit record, or CLI output, and two Exports of one
Volume have distinct identities.

## Layout

| Path | Contents |
| --- | --- |
| `src/` | Export controller, worker plan, argv renderer, effect port, colocated unit tests |
| `tests/` | hermetic Export lifecycle, sandbox, drain, and privacy conformance |
| `integration/` | virtiofsd launch and guest-mount fixtures |

## Commands

```bash
cd packages && cargo test -p d2b-provider-volume-virtiofs
cd packages && cargo clippy -p d2b-provider-volume-virtiofs --all-targets
```
