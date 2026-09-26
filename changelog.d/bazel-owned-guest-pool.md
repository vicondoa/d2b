---
type: feat
---

Build one guest per host-integration check, pool them, and run the lane as a single Bazel test

The type-10 lane had two guest images and eleven checks, and nine of the
checks declared a `nodes.machine` of their own - three a plain NixOS node
with no d2b daemon host at all, two turning nftables on, two installing
acceptance artifacts, and two booting a nested guest on the writable-store
shape. One image could not be all of those, so the checks that needed a guest
of their own were running against a guest that lacked what they needed.

Each check now gets a `guest_image` of its own, and the guest is built from
that check's own fixture rather than from a second copy of its node: the
guest-image evaluation reads the node, the machine size, the drive layout and
the evaluated assertions straight out of the fixture file, so there is no
inventory anywhere in the tree that can disagree with a check about which
guest it boots.

The lane test target owns the run. It groups the checks by the emulator
invocation they declare - not by closure, which is what makes two of them the
same shape - sizes a pool from the number of distinct invocations against the
budget the guest configurations declared and the memory, vCPUs and working
directory this host actually has, refuses before the pool is built if any
attached writable device cannot carry an internal snapshot, proves on a member
of every distinct invocation that a restored guest is the fresh boot its check
was written against, runs each check against a snapshot-restored copy of its
own guest, and retires rather than restores a member that has run a nested
guest. Each check reports under its own name in the lane's JUnit document, and
the lane's result is never cacheable.

The interpreter an unported check's script runs under is a declared runfile
from the pinned nix package set rather than whatever `python3` a developer's
shell happens to resolve.

The pool does not yet reach a check. A guest built from any of these
configurations mounts the host Nix store over 9p - the QEMU VM module's own
`mountHostNixStore`, which its direct-boot shape is built around - and QEMU
refuses an internal snapshot while a VirtFS export is mounted:
`Migration is disabled when VirtFS export path '...' is mounted in the guest
using mount_tag 'nix-store'`. The guest cannot boot without that export:
without it the activation's `/sysroot/nix/.ro-store` mount fails and the
system panics. So the guests are snapshottable and not snapshot-capable at
the same time, and every check fails at the snapshot rather than at an
assertion. Restoring from an external qcow2 overlay rather than an internal
`savevm` is the change that reconciles this with the restore-only rule; it is
not made here.
