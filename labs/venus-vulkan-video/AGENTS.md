# AGENTS.md - `labs/venus-vulkan-video/` isolation contract

This lab is an **experimental prototype**, not part of the d2b framework. It is
governed by this file *in addition to* the repo-root `AGENTS.md`. Where the two
conflict for files under `labs/venus-vulkan-video/`, this file wins.

Read this before changing anything in this directory.

## What this lab is

An attempt to make **stock, unmodified upstream Firefox** inside a guest VM
decode H.264 on the host NVIDIA GPU via `VK_KHR_video_decode_h264` forwarded
through Venus/virtio-gpu, replacing the current forked-Firefox + virtio-media
V4L2 path.

The work spans three upstream projects, forked under `vicondoa/`:

| Upstream | Fork |
|---|---|
| `gitlab.freedesktop.org/virgl/venus-protocol` | `vicondoa/venus-protocol-vulkan-video` |
| `gitlab.freedesktop.org/virgl/virglrenderer` | `vicondoa/virglrenderer-venus-vulkan-video` |
| `gitlab.freedesktop.org/mesa/mesa` | `vicondoa/mesa-venus-vulkan-video` |

## Hard rules

### 1. This lab is NOT production, and must never become production by accident

The code paths built here deserialize a **guest-controlled** Vulkan Video
command stream inside a host process holding an open GPU fd. Until the hardening
wave completes and an explicit production review passes, this must never be
enabled for any d2b VM whose guest is untrusted.

**Do not** wire any of this into `nixos-modules/`, add a `d2b.vms.<vm>.*`
option for it, or reference it from the root flake.

### 2. No host switch

Nothing here may require `nixos-rebuild switch`, an `/etc/nixos` edit, a new
systemd unit, or any persistent host configuration change. The lab runs entirely
from `nix build` outputs launched as the operator's own user.

The one privileged action is a **reversible, non-persistent** ACL grant on
`/dev/kvm` (`host/grant-kvm.sh`), auto-revoked by the launcher on exit. See
"Accepted exceptions" in the plan - this is a known, documented residual risk,
not a resolved one.

### 3. Complete isolation from the d2b control plane

The lab must never read or write `/etc/d2b`, `/var/lib/d2b`, the `d2bd` public
socket, or the privileged broker. It does not use d2b's VM lifecycle at all.

Note the lab **shares hardware** with any running d2b VMs (`/dev/kvm`, the
render node, `/dev/udmabuf`, RAM, the GPU). That is expected contention, and the
launcher warns about it. Stop live d2b VMs before taking measurements.

### 4. Nix source hygiene - mutable state lives OUTSIDE the repo

The repo-root `AGENTS.md` documents the `path:` fetcher hazard: a bare path
reference copies the **entire working tree** into the Nix store. A multi-GB
build directory inside `labs/` would be catastrophic for eval times.

Therefore:

- **All** mutable lab state lives at `${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/`
  - fork clones, build artifacts, writable disk overlays, guest profiles,
  evidence captures.
- Runtime state lives in a **per-run** directory under
  `$XDG_RUNTIME_DIR/venus-lab/<runid>/`.
- Nothing bulky or mutable is ever written inside `labs/`.
- Every eval/build resolves this flake as
  `git+file:///home/paydro/projects/d2b?dir=labs/venus-vulkan-video`.
- **Banned:** `path:` refs, bare `builtins.getFlake <path>`, and `src = self`
  for bulky sources.

Note `git ls-files --cached --others --exclude-standard` (used by the repo's
whole-tree policy lint) sees **untracked but non-ignored** files, so stray build
output inside `labs/` would be scanned as well as copied.

### 5. Host and guest package sets are strictly separate

Two different Mesa builds exist and must never be confused:

- **guest** → the patched lab Mesa (Venus ICD with video support)
- **host** → stock Mesa; only **virglrenderer** is patched

The flake exposes `hostPkgs` and `guestPkgs` separately so no global overlay can
cross the boundary. Do not introduce a top-level overlay that overrides Mesa.

### 6. Prove bindings, never assume them

Three things have silently-wrong failure modes and must be **proven**, not
assumed, with evidence recorded in `docs/`:

- **crosvm ↔ virglrenderer**: `crosvm.override { virglrenderer = labVirglrenderer; }`
  is a real relink. A `symlinkJoin` would still load nixpkgs' virglrenderer via
  RPATH. Prove with `ldd` / `patchelf --print-rpath` / `nix why-depends` and
  `LD_DEBUG=libs` or `/proc/<pid>/maps`.
- **guest ICD**: adding lab Mesa to the guest image does *not* make it the ICD in
  use. Prove with `readlink -f` of the ICD, `VK_LOADER_DEBUG=all`, and
  `vulkaninfo` driver/version.
- **sandbox capability**: the crosvm sidecar runs under bubblewrap. Prove
  capability **from inside the bwrap namespace** with
  `tests/host-caps.sh --in-sandbox`, never from the host shell.

#### Verified sandbox bind set

Established empirically in W0 by `tests/host-caps.sh`. Both `/sys` and the
extra NVIDIA nodes were **missing** from the original design and had to be added:

| Bind | Mode | Why |
|---|---|---|
| `/nix/store` | ro | closure; the NVIDIA ICD's `library_path` points here |
| `/run/opengl-driver` | ro | Vulkan ICD directory |
| `/etc` | ro | loader + `vulkan/icd.d` config |
| **`/sys`** | ro | **NVIDIA userspace enumerates the GPU via sysfs; without it `vkCreateInstance` SEGFAULTS inside the namespace** |
| `/proc` | fresh procfs | exposes `/proc/driver/nvidia` |
| `/dev/dri/renderD128` | dev-bind | render node |
| `/dev/nvidia0`, `/dev/nvidiactl` | dev-bind | core driver nodes |
| **`/dev/nvidia-modeset`, `/dev/nvidia-uvm`, `/dev/nvidia-uvm-tools`** | dev-bind | probed during instance creation |
| `/dev/udmabuf` | dev-bind | backs `external-blob` blob resources |

Two traps worth remembering, both hit during W0:

1. A `#!/usr/bin/env bash` shebang **fails inside the sandbox** (`/usr/bin` is not
   bound) with a misleading "No such file or directory" that looks like the
   script is missing. Invoke the interpreter by absolute path.
2. `sed ... | grep -q` under `set -o pipefail` **silently inverts results**:
   `grep -q` exits on first match, SIGPIPEs the upstream `sed`, and `pipefail`
   reports the pipeline as failed. Extract once into a variable instead.

### 7. Never bind the real compositor socket

The sandbox gets a **per-run nested compositor** (`cage`) socket, never the
operator's real `wayland-*` socket. A guest→renderer compromise must not reach
the real desktop session.

### 8. Upstream forks: append-only protocol IDs

`VK_COMMAND_TYPE_*` values are **explicitly assigned** in
`xmls/VK_EXT_command_serialization.xml` (345 values assigned as of
`base/70991d4`, max `345`, zero video entries). New video commands are appended
at **346+**.

`VN_WIRE_FORMAT_VERSION` **stays at 1**. Venus requires exact version equality
between guest and renderer, so bumping it would break every old guest. Video is
gated through the extension mask / capset instead. Any change that alters the
serialization of a pre-existing command is a **hard failure**.

## Testing

Per `tests/AGENTS.md`, this lab sits deliberately **outside** the d2b
Layer-1/Layer-2 test taxonomy:

- Protocol/generator/serialization/negative/fuzz tests live in the **upstream
  fork** test suites and are runnable as lab flake checks. Hermetic; must pass.
- VM/GPU/Firefox evidence tests are **manual hardware-tier** tests (they need the
  real T1000) and are excluded from root `make check` by design.
- **Do not** add a new top-level `tests/*.sh` to the d2b root - the repo test
  model forbids it.

## Codec scope - do not expand it speculatively

| Codec | Status |
|---|---|
| **H.264** | the only target |
| H.265 | available on the host, out of scope |
| VP9 | available on the host, out of scope - see the correction below |
| AV1 | **blocked at hardware** - Turing TU117/NVDEC4 has no AV1 decode engine |

**Correction.** This table previously said VP9 was "blocked at the host driver"
because no NVIDIA driver exposed `VK_KHR_video_decode_vp9`. The lab's own
measured host report contradicts that: driver 595.71.05 advertises the extension
at revision 1, reports `VIDEO_CODEC_OPERATION_DECODE_VP9_BIT_KHR` in the decode
queue's codec operations, sets `videoDecodeVP9 = true`, and lists three VP9
decode profiles. See `PINS.md` for the exact `evidence/host-caps-host.txt` line
references. The measurement is canon.

H.264 stays the only target regardless, because it is what the wire format
already carries and a second codec would widen the surface before the first one
has executed. YouTube is pinned to H.264 by configuration - a preference-only
measure, so Firefox stays unmodified. That pin is a **scope** decision, not a
host-capability one, and VP9 is deferred rather than impossible.

## Decision rule

If native Vulkan Video forwarding turns out to be blocked by a fundamental Venus
limitation, **stop and document the exact blocker**. Do not silently fall back to
translating Vulkan Video into V4L2, and do not invent a second architecture
without evidence that native forwarding is impossible.
