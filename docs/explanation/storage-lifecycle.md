# Storage lifecycle, restart, and synchronization

D2b treats host filesystem state as part of the control plane. The
state is not just a set of directories that happen to exist; it is the
evidence used to restart `d2bd`, re-adopt live runners, preserve TPM
and store-view identity, and decide whether a VM is healthy or degraded.

This document explains the design selected by
[ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md).
The exact schemas will live in reference docs and operator runbooks will live in
how-to docs as the implementation lands.

## Why this exists

The historical model split ownership among tmpfiles, activation scripts,
`d2bd`, the privileged broker, and runner processes. That made it easy
for two different layers to believe they owned the same inode. The common
symptoms were:

- a socket or state directory that worked only after a manual chmod or
  setfacl;
- a persistent file that disagreed with daemon memory;
- a stale runtime file that could not safely be deleted because a live
  process might still own it;
- a lock that appeared to move from a parent to a child process after
  fork/exec;
- broad default ACLs that fixed one role while accidentally widening
  another role's access.

The new model makes storage, restart, and synchronization explicit
contracts.

## The three contracts

### Storage contract

The storage contract lists every managed path, its owner, mode, ACLs,
lifecycle, cleanup rule, and repair authority. It answers "who owns this
inode, who may read/write it, when can it be removed, and what invariants
must never be violated?"

Examples of invariants that belong in the contract:

- `/etc/d2b` contains generated configuration and bundle artifacts; it
  is not runtime state.
- `/var/lib/d2b` holds persistent framework state.
- `/run/d2b` is boot-scoped runtime state, but a daemon restart does
  not make every entry stale.
- The store-view live pool is a hardlink farm and must never receive
  recursive chmod, chown, or setfacl operations.
- External paths such as `/run/user/<uid>` and `/dev/kvm` are grant-only
  surfaces; d2b may grant access or verify posture but does not own
  those roots.

### Restart contract

Every process role declares how restart works. A role is one of:

| Class | Meaning |
| --- | --- |
| `adoptable` | The process may survive daemon restart. D2b discovers it through its declared cgroup leaf, opens a fresh pidfd, verifies identity, and resumes supervision. |
| `recreatable` | The process can be restarted from persistent state without data loss. |
| `stateful-quarantine` | D2b cannot prove safety. It leaves the state alone and marks the component degraded. |
| `non-resumable` | The process cannot be resumed safely and requires an explicit restart/remediation. |
| `external-observed` | D2b observes health but does not own the process or resource. |

This is why runtime cleanup cannot be a broad sweep. `d2bd` may have
died while a VM and its role sockets stayed alive. The daemon must first
rebuild the live-owner set, then clean only entries whose owners are
proven dead.

### Synchronization contract

Locks are resources with owners. A lock row declares its path or in-memory
resource, lock primitive, allowed holder, transfer policy, release
authority, stale policy, and acquisition order.

New framework advisory file locks use OFD locks. Lock fds are close-on-exec
by default. A child does not become a lock owner just because it inherited
an fd; intentional transfer must be declared and recorded.

When several locks are needed, code must acquire them through the declared
total order. This prevents AB-BA deadlocks between VM lifecycle, StoreSync,
USBIP, storage reconciliation, and daemon-global operations.

## Broker authority and path safety

The broker is the privileged host mutation boundary. The daemon does not
send raw paths, owners, modes, or ACLs as authority. It sends opaque ids.
The broker resolves those ids against the trusted bundle and then walks
paths relative to declared roots with symlink, magic-link, and escape
defenses.

This matters because some parent directories are writable by daemon or
role identities. A compromised non-root process must not be able to swap a
symlink into a root-running repair path and trick the broker into changing
an unrelated host file.

## Degraded state instead of silent repair

When d2b cannot prove a safe action, it records a typed degraded state.
Examples include storage drift, ambiguous lock owner, adoption quarantine,
external dependency unhealthy, and migration failure.

The degraded ledger is diagnostic state, not broker authority. It is parsed
strictly, uses closed enums, and maps to static remediation ids. CLI output
surfaces the safe remediation command inline so operators do not have to
guess at chmod/chown commands.

## Observability without path leakage

Path hashes can be useful when diagnosing a local host, but raw paths can
contain sensitive context. The design therefore uses scope-salted hashes in
structured audit and local doctor output, not in metric labels. Metrics use
closed reason and scope classes.

Disk-image preparation follows the same fail-closed posture. A broker
`DiskInit` re-run does not treat path existence as sufficient evidence that a
raw image is mountable: it verifies the declared posture and ext4 superblock
before skipping, and formats an existing image only when kernel extent metadata
proves it is empty. This keeps host-side storage validation from letting a VM
boot into initrd emergency with an unmountable `/var` or writable-store image.

When a path is undeclared or malicious, d2b emits a separate
rate-limited incident event. Violation events have their own quota so an
attacker cannot flood the violation lane and evict normal audit history.

## Migration posture

The storage cutover is allowed to be disruptive once. During that planned
downtime, d2b may clear old boot-scoped runtime files and lock records
after proving VMs and runner processes are stopped. Persistent data such as
TPM NVRAM, store-view metadata, SSH keys, daemon adoption metadata, audit
history, host-runtime metadata, and disk images is preserved and verified.

After the cutover, normal restarts are continuation events again.
