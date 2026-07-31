#!/usr/bin/env python3
"""Drive the live cage Firefox to a local clip and report the decoder it chose.

Why not just look at the video playing
--------------------------------------
Firefox's fallback from Vulkan Video to VA-API to software is SILENT and the
picture is identical. "The video played" is therefore not evidence of anything.
This asks the running session what it actually did.

The two things that constitute proof, per the plan's evidence contract:
  - the decoder name Firefox selected for this element, and
  - that it is the Vulkan one rather than a software decoder.

Reused from the gate probe: Marionette framing is `<len>:<json>`, WebDriver
replies are wrapped in {"value": ...}, and chrome context needs
-remote-allow-system-access on the Firefox command line.
"""

import json
import socket
import sys
import time

HOST, PORT, TIMEOUT = "127.0.0.1", 2828, 60.0


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
        msg = self._recv()
        _, _, err, res = msg
        if err is not None:
            raise RuntimeError(f"{name}: {err}")
        if isinstance(res, dict) and set(res) == {"value"}:
            return res["value"]
        return res


def main():
    url = sys.argv[1] if len(sys.argv) > 1 else "file:///tmp/play.html"
    m = Marionette()
    m.send("WebDriver:NewSession", {"capabilities": {}})

    # Content context to navigate and query the media element.
    m.send("Marionette:SetContext", {"value": "content"})
    m.send("WebDriver:Navigate", {"url": url})

    # Let the media stack actually select a decoder and decode frames. A
    # screenshot-and-exit here is what made the W0 probe useless.
    time.sleep(20)

    # mozDecoderName / mozRequestDebugInfo are the authoritative statement of
    # what Firefox chose for THIS element -- not a global feature flag.
    script = """
      const v = document.getElementById('v');
      if (!v) { return JSON.stringify({error: 'no video element'}); }
      const out = {
        readyState: v.readyState,
        currentTime: v.currentTime,
        paused: v.paused,
        videoWidth: v.videoWidth,
        videoHeight: v.videoHeight,
        decodedFrames: (v.getVideoPlaybackQuality
                        ? v.getVideoPlaybackQuality().totalVideoFrames : null),
        droppedFrames: (v.getVideoPlaybackQuality
                        ? v.getVideoPlaybackQuality().droppedVideoFrames : null),
        mozDecoderName: v.mozDecoderName || null,
      };
      return JSON.stringify(out);
    """
    raw = m.send("WebDriver:ExecuteScript", {"script": script, "args": []})
    info = json.loads(raw)
    for k, v in info.items():
        print(f"{k}={v}")

    # mozRequestDebugInfo is a promise and carries the richer per-element view.
    dbg_script = """
      const resolve = arguments[arguments.length - 1];
      const v = document.getElementById('v');
      if (!v || !v.mozRequestDebugInfo) { resolve('{}'); }
      else { v.mozRequestDebugInfo().then(d => resolve(JSON.stringify(d)),
                                          e => resolve('{"error":"'+e+'"}')); }
    """
    try:
        raw = m.send("WebDriver:ExecuteAsyncScript",
                     {"script": dbg_script, "args": [], "scriptTimeout": 15000})
        dbg = json.loads(raw)
        vs = (dbg.get("videoState") or {}) if isinstance(dbg, dict) else {}
        for key in ("mVideoDecoderName", "mIsHardwareAccelerated",
                    "mVideoDecodeMode"):
            if key in vs:
                print(f"debug_{key}={vs[key]}")
        if not vs:
            print(f"debug_raw={json.dumps(dbg)[:400]}")
    except Exception as exc:  # noqa: BLE001
        print(f"debug_error={exc}")

    name = (info.get("mozDecoderName") or "").lower()
    hw = "vulkan" in name
    print(f"gate_firefox_vulkan_decode={'PASS' if hw else 'FAIL'}")
    return 0 if hw else 2


if __name__ == "__main__":
    sys.exit(main())
