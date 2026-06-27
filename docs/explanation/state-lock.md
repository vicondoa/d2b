# Why the daemon uses OFD state locks

Daemon-owned state still has to preserve the same
single-writer invariant that the legacy systemd/bash path relied on:
only one control-plane owner may act on the daemon itself, and only one
owner may manipulate a given VM at a time.

## Why OFD locks instead of classic POSIX advisory locks?

The daemon uses Linux **open-file-description (OFD) locks** via
`fcntl(F_OFD_SETLK)` rather than classic process-scoped POSIX advisory
locks.

That choice is deliberate:

- **OFD locks are tied to the file description, not the process.** A
  future supervisor fork can inherit the lock fd intentionally without
  having to re-negotiate ownership.
- **Closing an unrelated fd does not accidentally drop the lock.** With
  classic POSIX locks, one `close(2)` on the same inode can release all
  locks held by the process. That is the wrong failure mode for a daemon
  that may open the same lock path more than once.
- **Crash cleanup is automatic.** Once the last fd referencing the open
  file description disappears, the kernel releases the lock.

The lock file may still remain on disk after a crash; the **lock does
not**. The daemon therefore treats file presence as non-authoritative and relies
on the kernel lock state, not on stale filenames or mtimes.

## Lock files

The daemon uses two lock scopes:

- daemon singleton: `/run/d2b/daemon.lock`
  - owner/group: `root:d2bd`
  - mode: `0640`
- per-VM lock files: `/run/d2b/locks/<vm>.lock`

The daemon singleton lock prevents two `d2bd` instances from acting
as the same control-plane owner. The per-VM lock files carry the same
single-writer rule into lifecycle-affecting operations so a future
supervisor cannot race another actor on one VM while still allowing
independent VMs to proceed concurrently.

## Parent path invariant: no symlinked lock roots

The lock paths sit under `/run/d2b`, which means the parent path is
part of the trust boundary.

The daemon therefore validates that the parent directories are real directories,
not symlinks, before opening the lock file:

- `/run/d2b` must be owned by `root:root` and remain a real
  directory;
- `/run/d2b/locks` must likewise be a real directory;
- startup fails closed if a symlink swap is detected.

The point of this invariant is simple: if an attacker can redirect the
lock path, they can redirect the daemon's notion of exclusivity.

## Recovery semantics

### Daemon crash

If `d2bd` crashes, the kernel closes the daemon's last reference to
`/run/d2b/daemon.lock` and releases the OFD lock immediately. The
replacement daemon may then reopen the same path, reacquire the lock,
and continue. No manual “stale lock” deletion is required for correctness.

### Stale file cleanup

A stale **file** is not a stale **lock**. The daemon may truncate or recreate the
existing lock file once it has re-established the parent-directory
invariants, but it must never decide ownership from file existence
alone.

### Future supervisor fork inheritance

This is the main reason the daemon pays the OFD-lock complexity up front. If a
future supervisor model forks from the daemon and intentionally inherits
the lock fd, the lock stays associated with that open file description.
That lets the child take over supervision without a transient unlock
window. The converse also matters: if the parent exits but the child
still holds the inherited fd, the lock remains valid until the child
closes it.

In short: OFD locks model the ownership transfer the daemon may need later,
whereas classic POSIX locks model “one process, one lock table,” which
is the wrong abstraction for daemon/supervisor handoff.
