#!/usr/bin/env python3
"""YouTube end-to-end smoke test.

Deliberately a SMOKE TEST, not a benchmark. Adaptive bitrate, ads, network
variance and cache make YouTube unusable as a measurement; the numbers that
carry thresholds come from the deterministic local corpus instead.

What this establishes is only that the path survives contact with a real site:
a real player, real MSE buffering, real adaptive switching, and a codec chosen
by the site rather than by the test.

Codec pinning matters here. The lab disables WebM by policy so YouTube serves
H.264/MP4, because Venus carries only H.264 decode today. If YouTube served VP9
the decoder would correctly decline it and fall back to software -- which looks
identical on screen, which is why this reads the renderer's counters rather
than watching the video.

Usage: firefox-youtube.py [video-url] [seconds]
"""

import json
import socket
import sys
import time

HOST, PORT, TIMEOUT = "127.0.0.1", 2828, 90.0

# "Me at the zoo" -- the first video uploaded to YouTube. Chosen because it is
# permanently available, short, and has no age gate or region restriction, so
# the smoke test does not fail for reasons unrelated to decode.
DEFAULT_URL = "https://www.youtube.com/watch?v=jNQXAC9IVRw"


class Marionette:
    def __init__(self):
        self._s = socket.create_connection((HOST, PORT), timeout=TIMEOUT)
        self._s.settimeout(TIMEOUT)
        self._buf = b""
        self._id = 0
        self._recv()

    def _exact(self, n):
        while len(self._buf) < n:
            c = self._s.recv(65536)
            if not c:
                raise RuntimeError("connection closed")
            self._buf += c
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _recv(self):
        d = b""
        while True:
            c = self._exact(1)
            if c == b":":
                break
            d += c
        return json.loads(self._exact(int(d)).decode())

    def send(self, name, params=None):
        self._id += 1
        p = json.dumps([0, self._id, name, params or {}]).encode()
        self._s.sendall(str(len(p)).encode() + b":" + p)
        _, _, err, res = self._recv()
        if err is not None:
            raise RuntimeError(f"{name}: {err}")
        if isinstance(res, dict) and set(res) == {"value"}:
            return res["value"]
        return res


PROBE = """
  const vs = document.getElementsByTagName('video');
  if (!vs.length) { return JSON.stringify({error: 'no video element'}); }
  const v = vs[0];
  const q = v.getVideoPlaybackQuality ? v.getVideoPlaybackQuality() : {};
  return JSON.stringify({
    currentTime: v.currentTime,
    duration: v.duration,
    readyState: v.readyState,
    paused: v.paused,
    videoWidth: v.videoWidth,
    videoHeight: v.videoHeight,
    decodedFrames: q.totalVideoFrames || 0,
    droppedFrames: q.droppedVideoFrames || 0,
    src: (v.currentSrc || '').slice(0, 60),
  });
"""


def main():
    url = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_URL
    secs = int(sys.argv[2]) if len(sys.argv) > 2 else 45

    m = Marionette()
    m.send("WebDriver:NewSession", {"capabilities": {}})
    m.send("Marionette:SetContext", {"value": "content"})
    print(f"navigating to {url}")
    m.send("WebDriver:Navigate", {"url": url})

    # YouTube needs time to load the player, resolve an adaptive stream and
    # start MSE buffering. Sampling too early reads a player that has not
    # chosen a codec yet.
    # YouTube blocks autoplay without user interaction, so the player loads,
    # reaches readyState 4 and then sits paused at t=0. Measured exactly that:
    # a video session was created and zero decode commands followed. Muting
    # satisfies the autoplay policy.
    time.sleep(12)
    kick = """
      const resolve = arguments[arguments.length - 1];
      const vs = document.getElementsByTagName('video');
      if (!vs.length) { resolve('no video'); }
      else {
        const v = vs[0];
        v.muted = true;
        v.play().then(() => resolve('playing'), e => resolve('play rejected: ' + e));
      }
    """
    try:
        print("autoplay kick:",
              m.send("WebDriver:ExecuteAsyncScript",
                     {"script": kick, "args": [], "scriptTimeout": 15000}))
    except Exception as exc:  # noqa: BLE001
        print(f"autoplay kick failed: {exc}")

    # Track PEAK observed values, not the last sample. YouTube autoplays the
    # next video when one ends, and the final sample then lands on a fresh
    # element sitting paused at t=0 with zero frames -- which read as FAIL
    # immediately after a run that had demonstrably played.
    deadline = time.time() + secs
    peak = {"currentTime": 0.0, "decodedFrames": 0, "droppedFrames": 0}
    samples = 0
    while time.time() < deadline:
        time.sleep(10)
        try:
            raw = m.send("WebDriver:ExecuteScript", {"script": PROBE, "args": []})
            cur = json.loads(raw)
            if "error" in cur:
                continue
            samples += 1
            print(f"  t={cur['currentTime']:.1f}s "
                  f"{cur['videoWidth']}x{cur['videoHeight']} "
                  f"frames={cur['decodedFrames']} "
                  f"dropped={cur['droppedFrames']}")
            for k in ("currentTime", "decodedFrames", "droppedFrames"):
                # A torn-down element reports null for these, and comparing
                # None to a float raises -- which silently ended sampling
                # early on the first run that hit an autoplay transition.
                v = cur.get(k)
                if isinstance(v, (int, float)) and v > peak[k]:
                    peak[k] = v
            for k in ("videoWidth", "videoHeight", "duration", "src"):
                if cur.get(k):
                    peak.setdefault(k, cur[k])
        except Exception as exc:  # noqa: BLE001
            print(f"  probe error: {exc}")

    if not samples:
        print("RESULT: no successful probe")
        return 1

    for k, v in peak.items():
        print(f"peak_{k}={v}")

    dec, drop = peak["decodedFrames"], peak["droppedFrames"]
    if dec:
        print(f"drop_rate={drop / dec:.1%}")
    playing = peak["currentTime"] > 1.0 and dec > 30
    print(f"gate_youtube_playing={'PASS' if playing else 'FAIL'}")
    return 0 if playing else 2


if __name__ == "__main__":
    sys.exit(main())
