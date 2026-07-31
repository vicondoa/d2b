# Phase 0 - retiring the cheap unknowns

Everything here was measured before any video code was written, because all of
it was cheaper to answer than to assume, and one of the answers was a go/no-go
on the whole prototype.

Pinned revisions for every measurement below: virglrenderer `c44f6e50`, Mesa
`ae2e69ae`, venus-protocol `f81cb963`, nixpkgs `38a48874`. Guest kernel
6.18.40, Firefox 153.0, driver 595.71.05 on an NVIDIA T1000.

## 0.1 - Firefox's two gates: GO

**This was the highest-risk cheap unknown in the plan, and it gates the stated
goal independently of every line of decode work.**

Firefox refuses hardware video decode unless it is *already* GPU-rendering. The
plan records the pair as `LAYERS_WR && !UsingSoftwareWebRender` plus
`gfxVars::UseH264HwDecode()`. W0 listed both as unverified.

W0 could not have verified them. Its probe drove `firefox --headless`, and
headless never initialises WebRender at all, so the result - 0 WebRender
mentions, 0 Vulkan mentions (`serial.log:702-718`) - was not evidence that the
gates fail. It was evidence that the harness could not observe them in *any*
state. An absent signal from a probe that cannot produce the signal is not a
measurement.

Read from the live cage session instead, over Marionette, via
`Troubleshoot.snapshot()` - the exact data behind `about:support`:

| Gate | Result |
|---|---|
| `compositing` | `WebRender` (not `WebRender (Software)`) |
| `WEBRENDER` feature | `available`, `{'type': 'default', 'status': 'available'}` |
| **hardware WebRender** | **PASS** |
| `HARDWARE_VIDEO_DECODING_VULKAN` | `available` |
| **Vulkan hardware decode gate** | **PASS** |
| `gfx_adapterDescription` | `virgl (NVIDIA T1000/PCIe/SSE2)` |

Artifact: `evidence/firefox-gates-p0.txt`.

Firefox is GPU-rendering through Venus on the T1000 inside the nested cage
session, and the Vulkan decode feature is available. Both gates hold, so the
prototype's goal is reachable and the remaining risk is in the decode path.

### The near-miss worth recording

There is no `H264_HW_DECODE` feature in Firefox 153. There are two separate
features:

| Feature | Status | Meaning |
|---|---|---|
| `HARDWARE_VIDEO_DECODING` | `unavailable` | the generic VA-API path |
| `HARDWARE_VIDEO_DECODING_VULKAN` | `available` | the Vulkan Video path |

The generic one is blocklisted with `FEATURE_FAILURE_VIDEO_DECODING_TEST_FAILED`
- expected, because there is no VA-API driver in the guest for its probe to
succeed against. It is not the gate for this prototype's path.

The first version of the probe looked for `H264_HW_DECODE`, found nothing, fell
back to `HARDWARE_VIDEO_DECODING`, and reported **FAIL**. Read literally that
would have been a no-go on the entire plan, produced by a probe that was
looking for a key which does not exist in this build and then answering with a
feature governing a path the prototype deliberately does not use.

It was caught only because the probe dumps the *whole* feature log rather than
grepping for keys it expects. That is the same lesson W2 paid 23 panel rounds
for - a hand-written set deciding what to look at - arriving here as a
false negative rather than a false positive. Derive the surface; do not
enumerate it.

**Open question, deliberately not assumed either way.** Whether
`HARDWARE_VIDEO_DECODING = unavailable` also suppresses the Vulkan decoder is
not settled by this measurement. Only an actual decode attempt settles it, so
it is recorded and carried into the spike rather than resolved by reasoning.

## 0.2 - The current lock boots

The only successful boot capture in the tree was from the W0 package set
(`serial.log:442` names Mesa `lab-bcf312f`), while the lock had moved to Mesa
`ae2e69ae` and virglrenderer `c44f6e50`. The current combination had never been
run.

It boots: multi-user and graphical targets reached, Venus enumerated as
`Virtio-GPU Venus (NVIDIA T1000)`, ordinary Vulkan working.

This also discharges the deferred **E1c** criterion. E1c covered renderer commit
`53096e1c` (reject device extensions the renderer never advertised), which built
but had never executed because `/dev/kvm` required an interactive sudo password
at the time. `53096e1c` is an ancestor of `c44f6e50`, `/dev/kvm` now carries
`user:paydro:rw`, and the guest reaches graphical target with ordinary Venus
intact. `docs/blocked-e1c.md` is superseded.

### Baseline, re-confirmed at this lock

| Property | Value |
|---|---|
| Venus device | `Virtio-GPU Venus (NVIDIA T1000)`, `driverName = venus` |
| `VK_KHR_video_queue` on the Venus device | **absent** (0 matches) |
| `/dev/video*` | none |

Video is absent, which is what W2 asserts and what the spike must flip.

## 0.3 - A guest control channel

W0 shipped serial-only management, so every guest observation required a
predefined systemd unit and therefore a full image rebuild - one rebuild per
experiment for the remaining six waves.

passt now forwards host loopback to the guest sshd
(`-t 127.0.0.1/$SSH_PORT:22`). Password auth on the lab account is deliberate
and adds no exposure: the password is already a literal in
`guest/configuration.nix` and therefore already in the Nix store, and the
forward binds `127.0.0.1` only.

Probes are piped over stdin rather than baked into the image, so iterating on
one costs an SSH round trip instead of a rebuild and reboot. That was worth it
within the same phase - the gate probe took three iterations to get right.

## 0.4 - Two corrections

**VP9 is not blocked at the host driver.** `PINS.md` and the lab `AGENTS.md`
both asserted that no NVIDIA driver ships `VK_KHR_video_decode_vp9`, citing
zero device reports on `vulkan.gpuinfo.org`. The lab's own W0 measurement says
otherwise: revision 1 of the extension, `VIDEO_CODEC_OPERATION_DECODE_VP9_BIT_KHR`
in the decode queue's codec operations, `videoDecodeVP9 = true`, and three VP9
decode profiles. H.265 is present too. Both documents are corrected with the
evidence line references.

This changes no target. H.264 remains the only one, because it is what the wire
format already carries. What changes is the justification: pinning YouTube to
H.264 is a **scope** decision, not a host-capability one, and VP9 is deferred
rather than impossible. AV1 stays genuinely blocked - TU117/NVDEC4 has no AV1
decode engine whatever the driver advertises.

**The launcher silently reused a stale disk.** `run-lab-vm.sh` kept
`lab-disk.raw` across guest rebuilds while passing the newly built kernel,
initrd and init. That boots a new initrd against a disk holding the previous
closure, which fails as `Failed to start Find NixOS closure` and drops to
emergency mode - a message naming neither the disk nor the mismatch, and the
disk is the last thing an operator suspects because they did not change it. The
disk is now stamped with the store path it came from and re-materialised
whenever that path differs.

## Where this leaves the plan

The prototype's goal is reachable. Both Firefox gates hold, the lab boots at
the current lock, and the guest is scriptable.

Every remaining unknown is now in the decode path itself - DPB image sharing
across virtio-gpu, video session memory binding through Venus's memory path,
and decode queue family mapping. None is testable without executing a decode,
which is what the feasibility spike does next.
