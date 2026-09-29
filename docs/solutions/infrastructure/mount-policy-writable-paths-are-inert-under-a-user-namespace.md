---
title: "writable_paths in a launch MountPolicy is inert for any worker that runs under a user namespace"
date: 2026-09-29
category: infrastructure
module: d2b-broker spawn path
problem_type: security_issue
component: infrastructure
symptoms:
  - "A minted MountPolicy carries the correct writable path, and the grant is correct, but the process still cannot write"
  - "Nothing in the logs mentions the mount policy at all - no refusal, no error, just silence"
  - "The failure looks like a permissions bug, so the mount policy is the obvious thing to work on"
root_cause: missing_permission
resolution_type: code_fix
severity: high
tags: [sandbox, user-namespace, minijail, mount-policy, writable-paths, broker, posture]
---

# writable_paths in a launch MountPolicy is inert for any worker that runs under a user namespace

## Problem

A device worker was launched with a `MountPolicy` carrying a correctly authorized
`writable_paths` entry, and the state directory still was not writable inside
the sandbox. The policy was never applied at all - the broker skips the entire
bind-mount block for user-namespace postures, by design.

## Symptoms

- The minted policy contains the right path, resolved from trusted data, and it
  is correct. The process still cannot write.
- No log line mentions the mount policy. There is no refusal to trace, because
  nothing evaluated it.
- The symptom is `EACCES` (or a process that dies on its first write), which
  reads as a permissions problem and points straight at the mount policy.

## What Didn't Work

**Building a provider-declared storage grant to fix it.** The whole point of a
grant is that a provider declares the storage its worker needs and the
framework authorizes and materializes it. That mechanism was implemented,
reviewed, tested, and is a genuine, fail-closed capability - and it changed
nothing here, because the broker never consults the mount policy for this
posture. It was the largest single piece of work in the change set and it was
inert for the exact case it was written for.

**Treating the absence of a mount-policy error as "the policy was applied and
was insufficient".** Silence in this path means the code was skipped, not that
it succeeded quietly.

## Solution

The writability for a user-namespace worker comes from **host DAC/ACL**, not
from the mount policy. When a worker needs a writable directory, posture it on
the host and let the ACL do the work.

Two things in the tree already reflect that and are worth reading before
touching this area:

- `packages/d2b-broker/src/sys.rs:3248` - `in_ns_credentials = user_ns_spec.is_some()`
- `packages/d2b-broker/src/sys.rs:3351` - `if !in_ns_credentials { apply_mount_actions_debug(mount_actions) }`

The comment above the guard states the reason: the user-NS already provides
isolation, each namespace has its own mount tree cloned from `clone3`, and
bind-mounting onto inherited mounts *would fail* with `EPERM` because Linux
locks inherited mounts inside a user namespace. The minijail-style bind-mount
semantics are not merely unnecessary on the broker-pre-NS path, they are
actively wrong there.

The swtpm worker takes this path: `TPM_WORKER_NAMESPACES`
(`packages/d2b-core/src/bundle_resolver.rs:4881`) is
`device_namespaces(true, true, false, false, true)`, with `user_namespace: true`
in its posture table entry.

## Why This Works

The two sandbox models in this repo are genuinely different, and the mount
policy only participates in one of them:

| | non-user-NS (classic minijail) | user-NS (broker-pre-NS) |
|---|---|---|
| who builds the mount tree | the broker, pre-exec, via `apply_mount_actions` | the process itself, post-exec, with `CAP_SYS_ADMIN` inside the new NS root |
| `writable_paths` | **authoritative** | **not read** |
| writability source | `MountPolicy.writable_paths` | host DAC/ACL |
| symlink/`..` defences | broker-side `openat2` + fd-relative ops | the namespace boundary itself |

So for a user-namespace worker the question "how do I make this directory
writable?" has exactly one answer: **posture the directory on the host**. The
state-directory traverse grant and the per-Guest runtime directory creation both
work this way, and neither touches `writable_paths`.

## Prevention

**Before adding a `writable_paths` entry, check the posture's
`user_namespace`.** If it is set, the entry will be silently ignored. Ask
instead how the directory is postured on the host.

`writable_paths` is a security boundary. It is the right lever for a
classic-minijail posture and the wrong lever here - reaching for it on a
user-NS posture produces a policy that looks correct, passes review, and has no
effect.

**Treat a silently-ignored security control as worse than a missing one.** A
missing control shows up as a failure. A control that parses, validates, and is
then dropped at the last step gives every reader a false signal. If a launch
policy field can be inert for some posture, that fact belongs in the field's
documentation and in the posture table, not only in the spawn code.

**When a mount policy entry appears to have no effect, check whether the
`if !in_ns_credentials` guard was crossed before continuing to reason about the
path.** Everything downstream - is the path right, is it authorized, does it
exist - is a separate question from whether it was ever consulted.

## Related Issues

- #611 - TPM state Volume never provisions its per-Device directory
- `docs/solutions/infrastructure/posix-acl-mask-nullified-by-chmod-on-mode-0700-directories.md`
  - the same investigation's other root cause: the host ACL was the thing that
  mattered, and it had been silently nullified
