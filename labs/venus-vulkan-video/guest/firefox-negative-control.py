#!/usr/bin/env python3
"""Negative control: turn the Vulkan decoder off and reload the same clip.

The positive result (a renderer context appearing with hundreds of
vkCmdDecodeVideoKHR calls while Firefox plays) is only evidence if the same
playback produces NO such calls when the decoder is disabled. Otherwise the
counter could be attributing someone else's decode to Firefox.

Sets media.hardware-video-decoding-vulkan.enabled=false in the live session,
reloads, and plays for the same duration. The caller compares renderer decode
counters across the two runs.
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
        _, _, err, res = self._recv()
        if err is not None:
            raise RuntimeError(f"{name}: {err}")
        if isinstance(res, dict) and set(res) == {"value"}:
            return res["value"]
        return res


def main():
    enable = sys.argv[1] == "on" if len(sys.argv) > 1 else True
    m = Marionette()
    m.send("WebDriver:NewSession", {"capabilities": {}})

    # Prefs are chrome-context state.
    m.send("Marionette:SetContext", {"value": "chrome"})
    # BOTH prefs, because force-enabled overrides the Vulkan-specific one.
    # Flipping only the latter produced a clean "ok:false" while decode
    # continued at full rate -- a control that reported success and controlled
    # nothing.
    script = """
      const resolve = arguments[arguments.length - 1];
      const want = %s;
      const names = ["media.hardware-video-decoding-vulkan.enabled",
                     "media.hardware-video-decoding.force-enabled"];
      try {
        const got = [];
        for (const n of names) {
          Services.prefs.setBoolPref(n, want);
          got.push(n.split(".").pop() + "=" + Services.prefs.getBoolPref(n));
        }
        resolve("ok:" + got.join(" "));
      } catch (e) { resolve("err:" + e); }
    """ % ("true" if enable else "false")
    print("pref:", m.send("WebDriver:ExecuteAsyncScript",
                          {"script": script, "args": [], "scriptTimeout": 10000}))

    # Navigating away first forces the media element to be torn down and a new
    # decoder selected on return, rather than continuing with the one already
    # chosen under the previous pref value.
    m.send("Marionette:SetContext", {"value": "content"})
    m.send("WebDriver:Navigate", {"url": "about:blank"})
    time.sleep(2)
    m.send("WebDriver:Navigate", {"url": "file:///tmp/play.html"})
    time.sleep(20)

    raw = m.send("WebDriver:ExecuteScript", {"script": """
      const v = document.getElementById('v');
      if (!v) return JSON.stringify({error:'no video'});
      const q = v.getVideoPlaybackQuality ? v.getVideoPlaybackQuality() : {};
      return JSON.stringify({
        currentTime: v.currentTime, readyState: v.readyState,
        decodedFrames: q.totalVideoFrames || null,
        droppedFrames: q.droppedVideoFrames || null,
      });
    """, "args": []})
    for k, v in json.loads(raw).items():
        print(f"{k}={v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
