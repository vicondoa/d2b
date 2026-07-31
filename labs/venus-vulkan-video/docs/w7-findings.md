# W7 - YouTube end to end

> **CORRECTION - this document overstates the result.**
>
> YouTube loads, plays and drives real decode commands through Venus, and that
> much is measured. But the frames do not reach the screen: playback shows
> roughly half a second of video and then a flat dark green frame.
>
> The cause is a failing Vulkan-to-GL blit in Firefox's compositing path, not
> anything in Venus. See [`green-frame-finding.md`](./green-frame-finding.md).
>
> This also means the drop-rate numbers below measure a broken presentation
> path and should not be read as decode performance at all -- a second reason,
> beyond host contention, that they are not a benchmark.

**Result: YouTube plays in unmodified Firefox with H.264 decoded on the host
NVIDIA T1000 through Venus.** This is the prototype's stated goal.

## The measurement

Stock Firefox 153 in the guest cage session, driven to a YouTube watch page:

```
autoplay kick: playing
  t=10.1s   854x480   frames=304    dropped=41
  t=20.1s   854x480   frames=603    dropped=41
  t=30.2s  1280x720   frames=1206   dropped=534
  t=40.2s  1280x720   frames=1708   dropped=709
gate_youtube_playing=PASS
```

Renderer-side, over the same window:

| Signal | Delta |
|---|---|
| `vkCmdDecodeVideoKHR` commands | **+1026** |
| `vkCreateVideoSessionKHR` | **+6** |

Artifact: `evidence/w7-youtube-renderer-evidence.log`.

The adaptive switch from 854x480 to 1280x720 mid-playback is YouTube's own
bitrate ladder, and the decode path followed it without a new failure - the
session count rising alongside is the decoder being rebuilt for the new
resolution.

The codec pin holds: WebM is disabled by enterprise policy, so YouTube serves
H.264/MP4, which is the only codec Venus carries today. Without the pin YouTube
would prefer VP9, the decoder would correctly decline it, and playback would
fall back to software - identical on screen, which is why the renderer counters
are the measurement rather than the picture.

## What this is and is not

**It is a smoke test.** Adaptive bitrate, ads, network variance and cache make
YouTube unusable as a benchmark. Its value is that the path survives contact
with a real site: a real player, real MSE buffering, real adaptive switching,
and a codec chosen by the site rather than by the test.

**The drop rate is not a valid performance measurement.** 709 of 1708 frames
were dropped, concentrated entirely in the 720p segment - 41 dropped across the
first 603 frames at 480p, then 668 more once it switched to 720p.

That number was taken with the host under heavy load: seven of the operator's
d2b VMs running, several `rustc` jobs at ~100% CPU each, and 4 GB free of 62
GB. The lab's own contract says to stop live d2b VMs before taking
measurements, and the launcher warns about exactly this. Reporting 41.5% as a
property of the decode path would be attributing contention to the code.

What can be said: 480p was essentially clean (6.8%) and 720p degraded under
load. Whether 720p is clean on an idle host is **unmeasured**, and the honest
next step is to re-run it on a quiet machine rather than to explain the number.

## Carried forward

- **A real benchmark on the deterministic local corpus**, on an idle host, is
  still owed: guest and host CPU, dropped frames, time to first frame, GPU
  decode utilisation, and the extra GPU-copy cost, compared against software
  decode. That pair is directly comparable because both run the same stock
  Firefox 153.
- **The GPU-copy path is the target by design.** Direct DMA-BUF export stays
  off for the whole plan (`direct-export.enabled = false`), so some copy cost is
  expected and zero-copy is a follow-on rather than a success criterion. Whether
  that copy is what costs 720p under load is exactly what the idle-host
  measurement would separate.
- **The V4L2 comparison remains a legacy, non-comparable baseline.** It can
  only be measured with the Firefox 152 source fork, so it is a version *and*
  build-config difference. It is context, not a threshold.

## Small defects found and fixed

- The probe sampled the *last* observation rather than the peak. YouTube
  autoplays the next video when one ends, so the final sample landed on a fresh
  element paused at `t=0` with zero frames, and a run that had demonstrably
  played reported `FAIL`.
- A torn-down element reports `null` for `currentTime`, and comparing `None` to
  a float raised mid-loop, silently ending sampling early.
- Autoplay is blocked without user interaction: the player loaded, reached
  `readyState 4`, created a video session, and then sat paused at `t=0` with
  zero decode commands. Muting satisfies the policy.
