---
title: "A named ACL entry on a mode-0700 directory is silently nullified by any later chmod"
date: 2026-09-29
category: infrastructure
module: d2b-broker sandbox ACL grants
problem_type: security_issue
component: infrastructure
symptoms:
  - "A principal with a correct named ACL entry still gets EACCES on the directory"
  - "getfacl still lists the entry, so the grant looks applied"
  - "The child process exits 1 within ~200ms with no log written anywhere expected"
root_cause: missing_permission
resolution_type: code_fix
severity: high
tags: [acl, posix-acl, chmod, mask, sandbox, minijail, permissions]
---

# A named ACL entry on a mode-0700 directory is silently nullified by any later chmod

## Problem

`swtpm` could not open its log file in the guest and exited 1. The state
directory carried exactly the ACL entry it was supposed to have:

```
user::rwx
user:d2b-work-tpm0-swtpm:rwx
group::rwx
mask::rwx
other::---
```

The entry was present, the mask was intact, and access was still denied -
because the failure was one directory *up*, at the state root, whose ACL had
already been nullified.

## Symptoms

- A principal holding a correct named ACL entry gets `EACCES` anyway.
- `getfacl` on the failing directory shows `mask::---` and every named entry
  marked `#effective:---`, while the entries themselves are still listed.
- The worker's own error was `swtpm: Could not open logfile for writing:
  Permission denied` - the state *directory* was fine; the state *root* was not.
- No log, no socket, no NVRAM file was ever created, because the process died on
  its first filesystem write.

## What Didn't Work

**Reading the leaf's ACL and concluding the grant was correct.** The check
dumped the state directory, saw `rwx` for the swtpm principal, and the natural
conclusion was that the sandbox mount policy was wrong. It wasn't - the broker
skips `apply_mount_actions` entirely under a user namespace, so the mount policy
was never the lever here, and a large amount of work went into a grant mechanism
that turned out to be inert for this posture.

**Inferring the cause from the launch symptom.** The worker bound its ctrl socket
and then died on its server socket, which reads as a socket or permissions
problem. It was neither; the process never got far enough to bind anything.

**Treating `setfacl` as the final word.** `setfacl` succeeds, `getfacl` lists the
entry, and nothing in the normal flow re-checks it. The nullification happens
later, from a `chmod` issued by an unrelated code path.

## Solution

Two things had to be true: the traverse entry has to be present on the *root*,
and it has to stay *effective* after whatever else sets the mode.

`grant_swtpm_state_dir_traversal`
(`packages/d2b-broker/src/live_handlers.rs:2803`) now re-asserts the traverse
entry through `setfacl_required` (`:1320`) rather than tolerating a silent
`Ok(None)`, and the regression test
`swtpm_state_dir_traversal_is_effective_after_the_root_mode_is_reasserted`
(`:5646`) builds the exact failing shape - traverse entry installed on a 0700
root, declared mode re-asserted, asserted to *start* from a nulled mask - then
asserts the entry is effective afterwards.

The same failure mode was later found in a second place: the daemon's
`realize_serving_socket_dir` stamped a hardcoded `0700` on the shared per-Guest
runtime directory instead of the storage row's declared `1770`. Its guard was
also weaker than its own specification - it tested `mode & 0o7777 != 0`, a
*nonzero* test, where the spec required a *group-bits* test, so a declared `0700`
parsed cleanly. It now tests `mode & 0o070 != 0`, matching the broker's refusal.

## Why This Works

POSIX ACLs have a **mask** entry that is the ceiling on every named user and
named group entry. When a file's mode is changed by `chmod`, the kernel
recomputes that mask **from the file's group bits**. A mode of `0700` has group
bits `000`, so the mask becomes `---`, and every named entry below it is capped
to nothing - while remaining present in the file.

Reproduced on this host, exactly:

```
$ mkdir root && chmod 0700 root
$ setfacl -m u:12345:--x root
$ getfacl -p root | grep mask
mask::--x
$ chmod 0700 root
$ getfacl -p root | grep mask
mask::---
$ getfacl -p root | grep 12345
user:12345:--x	#effective:---
```

The entry survives. The permission does not. So `setfacl` after a `chmod` is
order-dependent: any `chmod` issued later - by a daemon, a tmpfiles rule, a
reconcile pass, or a mode re-assertion in a completely different code path -
silently undoes the grant, and the only symptom is `EACCES` at open time.

The consequence that made this expensive: a *correct grant one level below a
root whose traverse access was nullified is no grant at all*. Checking the
directory you care about tells you nothing about whether it is reachable.

## Prevention

**When a directory is mode 0700 and relies on named ACL entries, never re-assert
the mode with `chmod`.** Re-apply the ACL after any mode change, or use a mode
with non-zero group bits if the grant is meant to survive a `chmod`.

`1770` works for this reason: it has group bits, so an `fchmod` to it recomputes
the mask to `rwx` and every named entry stays effective. `0700` does not.

**Assert effectiveness, not presence.** A test that greps `getfacl` for the
entry's presence passes while the permission is dead. Assert the mask, or assert
that no entry is `#effective`-downgraded:

```rust
// The entry must be EFFECTIVE, not merely present - the pre-fix shape
// passes a presence check and fails this one.
let mask = /* parse getfacl -p output */;
assert_ne!(mask, 0o000, "named entries are capped to nothing");
```

**Check the whole path, not just the leaf.** Traverse access on every ancestor is
part of the grant. A dump of the leaf alone is what made this invisible for
hours: a perfect `rwx` grant one level below a root that had none.

**Treat a mode re-assertion as a privileged operation.** Any code path that
re-applies a declared mode to a directory with ACLs must re-apply the ACLs too,
or refuse when it cannot. This bit twice in one change set, on two different
directories, in two different crates.

## Related Issues

- #611 - TPM state Volume never provisions its per-Device directory
- #612 - device-worker-launch: swtpm worker dies before main(), leaving no log
- #615 - controller-session failure is undiagnosable (same class of "the log
  does not say what actually happened")
