# E1c - discharged

**Superseded by [`phase0-findings.md`](./phase0-findings.md) §0.2.**

E1c covered renderer commit `53096e1c` (reject device extensions the renderer
never advertised). It built but had never executed, because obtaining
`/dev/kvm` required an interactive sudo password that was not available at the
time.

That constraint is gone. `/dev/kvm` now carries `user:paydro:rw`, `53096e1c` is
an ancestor of the pinned `c44f6e50`, and the guest boots to graphical target
at that revision with ordinary Venus intact and video extensions absent.

The original runtime criteria are all met:

| Criterion | Result |
|---|---|
| guest reaches graphical target | yes |
| ordinary Venus device still works | yes - `Virtio-GPU Venus (NVIDIA T1000)`, `driverName = venus` |
| video extensions remain absent | yes - 0 matches for `VK_KHR_video_queue` |

One correction to the original note: it said `prove-guest-icd` would report the
runtime `deviceName` and `driverName`. That app does not run the VM or
`vulkaninfo`; it inspects the Mesa package and its source. The runtime evidence
comes from the guest itself, now over the SSH control channel.
