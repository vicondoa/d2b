#!/usr/bin/env python3
"""Read the live Firefox session's gfx and media gates over Marionette.

Why this exists
---------------
Firefox refuses hardware video decode unless it is ALREADY GPU-rendering:
the decision is `LAYERS_WR && !UsingSoftwareWebRender` plus
`gfxVars::UseH264HwDecode()`. Both are runtime state of the real session, not
build configuration, so they cannot be read from the package, from prefs, or
from a `--headless` process.

The W0 baseline probed a headless Firefox and reported zero WebRender and zero
Vulkan mentions. That was not evidence that the gates fail -- it was evidence
that headless never initialises WebRender at all, so the probe could not have
observed them either way. This talks to the cage session instead.

Protocol
--------
Marionette frames messages as `<byte-length>:<json>`. After connecting, the
server sends an unsolicited handshake; every subsequent exchange is
`[0, msgid, name, params]` out and `[1, msgid, error, result]` back.

Output is `key=value` lines so callers can grep without a JSON parser. The
ENTIRE gfx feature log is dumped rather than a guessed subset: feature key
names have moved between releases, and a probe that greps for a key that no
longer exists reports "absent" indistinguishably from "disabled".
"""

import json
import socket
import sys

HOST = "127.0.0.1"
PORT = 2828
TIMEOUT = 60.0

# Troubleshoot.snapshot() is promise-returning in current Firefox and
# callback-taking in older ones. Support both so the probe does not silently
# report UNREACHABLE on a version skew.
SNAPSHOT_JS = r"""
const resolve = arguments[arguments.length - 1];
try {
  const { Troubleshoot } =
    ChromeUtils.importESModule("resource://gre/modules/Troubleshoot.sys.mjs");
  const r = Troubleshoot.snapshot(s => resolve(JSON.stringify(s)));
  if (r && typeof r.then === "function") {
    r.then(s => resolve(JSON.stringify(s)),
           e => resolve("ERROR:" + (e && e.message ? e.message : e)));
  }
} catch (e) {
  resolve("ERROR:" + (e && e.message ? e.message : e));
}
"""


class MarionetteError(RuntimeError):
    pass


class Marionette:
    def __init__(self, host=HOST, port=PORT, timeout=TIMEOUT):
        self._sock = socket.create_connection((host, port), timeout=timeout)
        self._sock.settimeout(timeout)
        self._buf = b""
        self._msgid = 0
        self._recv()  # drain handshake so it is not read as a command reply

    def _recv_exact(self, n):
        while len(self._buf) < n:
            chunk = self._sock.recv(65536)
            if not chunk:
                raise MarionetteError("connection closed by Firefox")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _recv(self):
        # Length prefix is ASCII digits terminated by ':'. Read one byte at a
        # time so we never consume into the following frame.
        digits = b""
        while True:
            c = self._recv_exact(1)
            if c == b":":
                break
            if not c.isdigit():
                raise MarionetteError(f"bad length prefix byte {c!r}")
            digits += c
        return json.loads(self._recv_exact(int(digits)).decode("utf-8"))

    def _send(self, name, params=None):
        self._msgid += 1
        payload = json.dumps([0, self._msgid, name, params or {}]).encode("utf-8")
        self._sock.sendall(str(len(payload)).encode("ascii") + b":" + payload)
        msg = self._recv()
        if not isinstance(msg, list) or len(msg) != 4:
            raise MarionetteError(f"unexpected frame: {msg!r}")
        _, _, err, result = msg
        if err is not None:
            raise MarionetteError(f"{name} failed: {err}")
        # WebDriver commands wrap their return in {"value": ...}; Marionette's
        # own commands do not. Unwrap only the wrapper, never a payload that
        # legitimately happens to be a dict.
        if isinstance(result, dict) and set(result) == {"value"}:
            return result["value"]
        return result

    def new_session(self):
        return self._send("WebDriver:NewSession", {"capabilities": {}})

    def snapshot(self):
        """Troubleshoot.snapshot() -- the exact data behind about:support."""
        self._send("Marionette:SetContext", {"value": "chrome"})
        raw = self._send(
            "WebDriver:ExecuteAsyncScript",
            {"script": SNAPSHOT_JS, "args": [], "scriptTimeout": 30000},
        )
        if isinstance(raw, str) and raw.startswith("ERROR:"):
            raise MarionetteError(raw[6:])
        return json.loads(raw)

    def close(self):
        try:
            self._sock.close()
        except OSError:
            pass


def main():
    try:
        m = Marionette()
        m.new_session()
        snap = m.snapshot()
    except Exception as exc:  # noqa: BLE001 - this is a probe; report, don't raise
        print("marionette=UNREACHABLE")
        print(f"marionette_error={type(exc).__name__}: {exc}")
        return 1

    print("marionette=OK")
    gfx = snap.get("graphics", {}) or {}

    # Dump every feature the build knows about, with its status. This is the
    # authoritative answer for both gates and it does not depend on this script
    # knowing the current key spelling.
    features = (gfx.get("featureLog", {}) or {}).get("features", []) or []
    print(f"feature_count={len(features)}")
    statuses = {}
    for entry in features:
        name = entry.get("name", "?")
        status = entry.get("status", "?")
        statuses[name] = status
        print(f"feature[{name}]={status}")

    # Compositor identity. Reported under different keys across releases, so
    # try each and print what was actually found.
    compositor = None
    for key in ("compositing", "webrenderCompositor", "windowLayerManagerType"):
        if gfx.get(key):
            compositor = gfx[key]
            print(f"compositing_source={key}")
            break
    print(f"compositing={compositor}")

    def ok(status):
        return isinstance(status, str) and status.lower().startswith("available")

    # Gate 1: hardware WebRender. Software WebRender also reports WEBRENDER as
    # available, so the compositor string is what distinguishes them.
    comp = (compositor or "").lower()
    hw_webrender = "webrender" in comp and "software" not in comp
    if not features and compositor is None:
        hw_webrender = False
    print(f"gate_hardware_webrender={'PASS' if hw_webrender else 'FAIL'}")

    # Gate 2: the hardware-decode gate for the VULKAN path specifically.
    #
    # Firefox 153 has no H264_HW_DECODE feature. It has two separate ones:
    #
    #   HARDWARE_VIDEO_DECODING         the generic (VA-API) path
    #   HARDWARE_VIDEO_DECODING_VULKAN  the Vulkan Video path
    #
    # They are separate features because they are separate paths, and in this
    # guest the generic one is blocklisted with
    # FEATURE_FAILURE_VIDEO_DECODING_TEST_FAILED -- which is expected, because
    # there is no VA-API driver in the guest for its probe to succeed against.
    # Reading that as the answer would report a blocking failure for a path the
    # prototype does not use and is deliberately not building.
    #
    # Both are reported. The Vulkan one is the gate; the generic one is
    # context, because whether its failure also suppresses the Vulkan decoder
    # is a real question that only an actual decode attempt can settle.
    vulkan_dec = statuses.get("HARDWARE_VIDEO_DECODING_VULKAN")
    generic_dec = statuses.get("HARDWARE_VIDEO_DECODING")
    legacy_h264 = statuses.get("H264_HW_DECODE")
    vulkan_ok = ok(vulkan_dec)
    print(f"hwdec_vulkan_status={vulkan_dec}")
    print(f"hwdec_generic_status={generic_dec}")
    print(f"legacy_H264_HW_DECODE_status={legacy_h264}")
    print(f"gate_hardware_video_decoding_vulkan={'PASS' if vulkan_ok else 'FAIL'}")

    for label in (
        "adapterDescription",
        "adapterVendorID",
        "adapterDeviceID",
        "adapterDriverVersion",
        "isGPU2Active",
    ):
        print(f"gfx_{label}={gfx.get(label)}")

    media = snap.get("media", {}) or {}
    print(f"media_supported_decoders={media.get('supportedDecoders')}")

    # Per-feature reasoning, which is where a FAIL explains itself.
    for entry in features:
        log = entry.get("log") or []
        if log and entry.get("name") in (
            "WEBRENDER",
            "WEBRENDER_COMPOSITOR",
            "HARDWARE_VIDEO_DECODING",
            "HARDWARE_VIDEO_DECODING_VULKAN",
        ):
            print(f"--- {entry.get('name')} log ---")
            for line in log[:10]:
                print(f"  {line}")

    m.close()
    return 0 if (hw_webrender and vulkan_ok) else 2


if __name__ == "__main__":
    sys.exit(main())
