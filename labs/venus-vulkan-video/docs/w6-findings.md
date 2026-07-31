# W6 - unmodified Firefox decodes H.264 on the host GPU through Venus

> **CORRECTION - this document overstates the result.**
>
> The headline below says unmodified Firefox decodes H.264 through Venus. The
> decode part is true and measured. What is NOT true is any implication that
> playback works: the decoded frames never reach the screen, and the video area
> renders as a flat dark green after about half a second.
>
> A single GL blit in Firefox's MediaPDMDecoder context fails with
> `GL_INVALID_OPERATION`, virglrenderer marks the context as having submitted
> an illegal command buffer, and all 11,270 subsequent submissions to it are
> refused. Zero of those errors come from the Venus path.
>
> See [`green-frame-finding.md`](./green-frame-finding.md). The evidence in
> this document is correct about what it measured; it measured the wrong thing
> to support the claim it made.

**Result: it works, with a passing negative control.**

Stock upstream Firefox 153, zero source patches, running in the guest cage
session, decodes H.264 on the host NVIDIA T1000 via `VK_KHR_video_decode_h264`
forwarded through Venus/virtio-gpu - while being GPU-rendered through the same
virtio-gpu device.

## The measurement

| Condition | Frames decoded | Renderer sessions | Renderer decode commands |
|---|---|---|---|
| Firefox playing, Vulkan decode **on** | 599-600 | **+3** | **+448 … +1024** |
| Firefox playing, Vulkan decode **off** | **600** | **0** | **0** |
| Firefox on `about:blank` | - | **0** | **0** |

Artifacts: `evidence/w6-firefox-renderer-evidence.log`,
`evidence/firefox-gates-p0.txt`.

**The frame count is identical in both playback rows.** That is the entire
reason this wave needs command-level evidence: Firefox's fallback from Vulkan
Video to software is silent, produces the same picture, the same frame count,
and the same `readyState`. "The video played" is not evidence of anything, and
neither is `about:support` on its own.

The three rows together are what make the claim stand up:

- **Positive** - decode commands appear in a renderer context that did not
  exist before Firefox was pointed at the clip.
- **Negative** - with the decoder disabled before Firefox starts, the same
  clip plays with zero renderer video activity. So the commands are caused by
  the feature, not merely coincident with playback.
- **Idle** - with Firefox open but on `about:blank`, zero. So the commands are
  caused by Firefox decoding, not by something else in the guest.

## Proving Firefox is unmodified

The prototype's premise is stock Firefox, so this is proven by derivation
identity rather than asserted:

```
lab guest firefox-unwrapped: /nix/store/43kfkhgp6ngli81w23p42cwlx90ql07x-firefox-unwrapped-153.0.drv
stock nixpkgs 38a48874:      /nix/store/43kfkhgp6ngli81w23p42cwlx90ql07x-firefox-unwrapped-153.0.drv
```

Identical `.drv` path means identical source, patch list and build inputs - a
source patch anywhere would change the hash. The guest image closure contains
exactly one Firefox (`firefox-unwrapped-153.0`) plus a `wrapFirefox` wrapper
adding only enterprise *preferences*.

For contrast, the existing V4L2 path this replaces uses a **different** Firefox:
version 152.0, built with `.override` and source patches to
`FFmpegVideoDecoder.cpp`. It is untouched and remains the rollback path.

## The instrumentation

The counters live in the renderer's video dispatch
(`vkr_dispatch_vkCmdDecodeVideoKHR`, `vkr_dispatch_vkCreateVideoSessionKHR`)
and log on a curve - first three calls, then powers of two - so an experiment
is observable immediately without a long playback flooding the log.

They are the only point in the stack a fallback cannot satisfy. Everything
above fails silently: FFmpeg falls back and exits 0, Firefox falls back and
plays an identical picture, and `mozDecoderName` is ChromeOnly so it reads as
`None` from content context regardless of what was used.

## Three false readings this wave produced

Each looked like a result and was not. Recording them because each is a shape
that could recur.

### 1. A locked pref made the claim unfalsifiable

The first negative control set
`media.hardware-video-decoding-vulkan.enabled=false`, read it back as `true`,
and playback continued. The pref was `Status = "locked"` in the Firefox
enterprise policy.

Locking reads like rigour - the feature is guaranteed on and cannot be
perturbed - but it silently refuses the write that the control depends on. The
control appeared to run and the result read as "decode happens either way",
which is a false pass dressed as a measurement.

### 2. Unlocking one pref was not enough

With the Vulkan pref unlocked and confirmed `false`, decode still ran at full
rate: 512 commands, 3 sessions. `media.hardware-video-decoding.force-enabled`
was still locked `true`, and force-enable overrides the normal gating.

`force-enabled` is deliberately on, because the generic
`HARDWARE_VIDEO_DECODING` feature is blocklisted in the guest by a VA-API probe
that cannot succeed there - the guest has no VA-API driver. The fix was to keep
its default `true` but unlock it, so the control can move it.

### 3. Runtime pref changes are inert

With **both** prefs confirmed `false` at runtime, decode *still* ran unchanged.
The decoder module reads these once when it initialises; toggling them
mid-session does not re-evaluate.

The working control therefore persists the prefs to the profile and restarts
the session, so they are in force when Firefox next starts. This is why the
`about:blank` idle test mattered - it isolated Firefox as the source
independently of the prefs, and made it clear the prefs were the problem rather
than the attribution.

### And one flaw in the measurement itself

`decode_total()` originally took the maximum `decode_cmds` across the whole
log. The counter is a file-static in the render-server process, so every new
context starts again at 1 - meaning once one context reported 2048, a fresh
context climbing to 512 did not move the maximum, and a genuine positive read
as `delta 0`. Observed while restoring the positive phase. It now sums
per-context maxima.

The session-create count was unaffected, because those are distinct log lines
and the count is monotonic. That is why the session delta was the signal that
stayed trustworthy throughout.

## What is still outstanding

- **YouTube end to end** (W7). This wave used a deterministic local corpus by
  design: adaptive bitrate, ads, network and cache make YouTube unusable as a
  measurement, so it is the smoke test rather than the evidence.
- **Frames staying GPU-backed for presentation.** The evidence contract asks
  for proof that decoded images reach the compositor via GPU import and are not
  round-tripped through host-visible staging. `vkCmdDecodeVideoKHR` plus
  hardware WebRender do not jointly imply it. Not yet measured.
- **All of the hardening.** This runs on the spike renderer, which forwards
  guest input the W3 plan requires to be validated.
