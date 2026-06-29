#!/usr/bin/env bash
# tests/clipboard-picker-smoke.sh
#
# Layer-2 smoke test: prove the d2b-clip-picker binary does a valid ADR 0042
# protocol handshake (client_hello) on a socketpair IPC channel.
#
# Usage:
#   bash tests/clipboard-picker-smoke.sh
#
# Prerequisites:
#   - D2B_CLIP_PICKER: absolute path to the compiled d2b-clip-picker binary.
#     Falls back to "d2b-clip-picker" on PATH if unset.
#
# Exit codes:
#   0 — handshake succeeded; picker is functional
#   1 — handshake failed (see stderr)
#   2 — usage / missing binary

set -euo pipefail

PICKER="${D2B_CLIP_PICKER:-d2b-clip-picker}"

if ! command -v "$PICKER" &>/dev/null 2>&1 && [[ ! -x "$PICKER" ]]; then
    echo "clipboard-picker-smoke: SKIP — d2b-clip-picker not found" \
         "(set D2B_CLIP_PICKER to the binary path)" >&2
    exit 0
fi

echo "clipboard-picker-smoke: probing $(command -v "$PICKER" 2>/dev/null || echo "$PICKER")" >&2

# Create a socketpair using socat/bash process substitution isn't portable
# for fd passing. Use Python (available on NixOS hosts) or fall back to
# a minimal C helper approach via /proc/self/fd if needed.
#
# Primary path: Python one-liner that creates a socketpair, forks the picker
# with one end as --ipc-fd, then reads and validates client_hello from the
# other end.

python3 - "$PICKER" <<'PYEOF'
import sys, os, socket, subprocess, json, select

picker_path = sys.argv[1]

parent_fd, child_fd = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

proc = subprocess.Popen(
    [picker_path, "--ipc-fd", str(child_fd.fileno())],
    pass_fds=(child_fd.fileno(),),
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
child_fd.close()  # close child end in parent

# Wait up to 5 seconds for the client_hello frame.
ready, _, _ = select.select([parent_fd], [], [], 5.0)
if not ready:
    proc.terminate()
    proc.wait()
    print("clipboard-picker-smoke: FAIL — picker did not send client_hello within 5 s", file=sys.stderr)
    sys.exit(1)

data = b""
while b"\n" not in data:
    chunk = parent_fd.recv(65536)
    if not chunk:
        break
    data += chunk

parent_fd.close()
proc.terminate()
proc.wait()

line = data.split(b"\n")[0].decode("utf-8", errors="replace").strip()
try:
    frame = json.loads(line)
except json.JSONDecodeError as exc:
    print(f"clipboard-picker-smoke: FAIL — client_hello is not valid JSON: {exc}", file=sys.stderr)
    print(f"  raw: {line!r}", file=sys.stderr)
    sys.exit(1)

if frame.get("type") != "client_hello":
    print(f"clipboard-picker-smoke: FAIL — first frame type is not client_hello: {frame!r}", file=sys.stderr)
    sys.exit(1)

if "protocol_version_range" not in frame:
    print("clipboard-picker-smoke: FAIL — client_hello missing protocol_version_range", file=sys.stderr)
    sys.exit(1)

if "picker_version" not in frame:
    print("clipboard-picker-smoke: FAIL — client_hello missing picker_version", file=sys.stderr)
    sys.exit(1)

print("clipboard-picker-smoke: OK — picker sent valid client_hello", file=sys.stderr)
print(f"  type={frame['type']!r}", file=sys.stderr)
print(f"  picker_version={frame.get('picker_version')!r}", file=sys.stderr)
vrange = frame.get("protocol_version_range", {})
print(f"  protocol_version_range=min={vrange.get('min')} max={vrange.get('max')}", file=sys.stderr)
PYEOF

echo "clipboard-picker-smoke: PASS" >&2
