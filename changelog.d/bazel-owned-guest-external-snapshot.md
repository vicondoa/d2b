---
type: fix
---

Restore a pool member from an external qcow2 overlay instead of an internal snapshot

The lane's equivalence gate refused every check at the same line: the
emulator will not take an internal snapshot while a VirtFS export is mounted
in the guest, and every lane guest mounts one, because it is booted from a
host store path and panics at activation without it. The gate now takes an
*external* snapshot - a qcow2 overlay per writable device, taken with
`blockdev-snapshot-sync` - and a restore drops the layer the check dirtied,
takes a fresh one over the frozen image, and restarts the guest on it.

The restore is disk-only, and the gate says so rather than claiming a
RAM+disk restore it no longer performs: what it proves is that a guest booted
from the snapshot-restored disk matches the fresh boot the check's assertions
were written against. Every writable device is covered, including the state
disk the node attached through its option list, because `/var/lib/d2b` holds
the daemon's store and a snapshot that left it out would hand the second
check on a member the first check's rows. The guest's own drives are attached
as block nodes rather than as drives, which is what lets a restore take the
device off its dirty layer and put it on a clean one.
