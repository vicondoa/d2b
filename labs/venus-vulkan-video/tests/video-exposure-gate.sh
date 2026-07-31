#!/usr/bin/env bash
# Assert the renderer advertises EXACTLY the video support it implements.
#
# INVERTED IN W3, not deleted. The gate's headers used to propose deleting it
# in the commit that flipped exposure. That was the wrong instinct: a wave that
# spent 23 rounds on gates crediting the wrong thing should not hand off
# between two gates with no overlap, and more importantly the ENCODE half of
# this gate is still exactly as load-bearing as it was. Deleting it would
# remove the encode assertion at precisely the moment decode goes live and
# encode becomes the only thing standing between the guest and an
# unimplemented surface.
#
# Three conditions now:
#
#   1. Enabled VK_KHR_video_* extensions are exactly the DECODE allowlist. Too
#      few and decode silently stops working; too many and the renderer is
#      promising something it cannot execute.
#
#   2. Every enabled extension's commands have a non-NULL dispatch entry. The
#      capset is read as a promise the renderer can execute the WHOLE
#      extension, so an advertised extension with a NULL entry is a decoder the
#      guest can reach and the renderer cannot serve.
#
#   3. NO encode extension is enabled and NO encode command is dispatched.
#      Carried over verbatim from the pre-W3 gate. Encode is unimplemented; a
#      NULL dispatch entry is what makes an unexpected encode command fail
#      closed rather than execute.
#
# Usage: video-exposure-gate.sh [<virglrenderer-dir>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${1:-${VIRGL_DIR:-$STATE/virglrenderer}}"

die() { echo "exposure-gate: $*" >&2; exit 1; }

VENUS="$VIRGL_DIR/src/venus"
[ -d "$VENUS" ] || die "no src/venus in $VIRGL_DIR"

status=0

# --- 1. extension allowlist -------------------------------------------------

table_file="$VENUS/vkr_common.c"
[ -f "$table_file" ] || die "no $table_file"

# The table is a designated initializer; a video entry would read
# `.KHR_video_queue = true,`. Comments are stripped so a commented-out line
# cannot trip the gate, and `= false` is allowed so an explicit disable is
# expressible.
#
# Matching the literal token `true` was a hand-written set of accepted
# spellings, which is this wave's recurring root cause. `.KHR_video_queue = 1`
# is legal C against a bool field, enables the extension, and passed the gate.
# So match ANY assignment to a video field and allow only the explicitly
# disabling values; every other spelling fails closed.
enabled_video=$(
  sed -e 's://.*::' -e 's:/\*[^*]*\*\+\([^/*][^*]*\*\+\)*/::g' "$table_file" \
    | grep -oE '\.KHR_video_[a-z0-9_]+[[:space:]]*=[[:space:]]*[^,;}]+' \
    | grep -vE '=[[:space:]]*(false|0)[[:space:]]*$' \
    | grep -oE '^\.KHR_video_[a-z0-9_]+' | sed 's/^\.//' | sort -u || true
)

# The allowlist. Adding a name here is a claim the renderer implements and
# wires EVERY command of that extension -- see condition 2.
expected_video="KHR_video_decode_h264
KHR_video_decode_queue
KHR_video_queue"

if [ "$enabled_video" != "$expected_video" ]; then
  echo "exposure-gate: FAIL -- enabled video extensions are not the decode allowlist" >&2
  diff <(printf '%s\n' "$expected_video") <(printf '%s\n' "$enabled_video") \
    | sed 's/^/  /' >&2 || true
  status=1
else
  echo "exposure-gate: vkr_extension_table enables exactly the decode allowlist"
fi

# Condition 3a: no ENCODE extension enabled. Unchanged in meaning from the
# pre-W3 gate, and now the only half of it still asserting absence.
enabled_encode=$(printf '%s\n' "$enabled_video" | grep -E 'encode' || true)
if [ -n "$enabled_encode" ]; then
  echo "exposure-gate: FAIL -- an ENCODE extension is enabled:" >&2
  printf '%s\n' "$enabled_encode" | sed 's/^/  /' >&2
  status=1
else
  echo "exposure-gate: no video ENCODE extension is enabled"
fi

# --- 2. dispatch entries ----------------------------------------------------

# A populated entry looks like `dispatch->dispatch_vkCmdDecodeVideoKHR = ...`.
# Searching the whole venus directory rather than a known file, so moving the
# assignment somewhere else does not evade the check.
# A populated entry looks like `dispatch->dispatch_vkCmdDecodeVideoKHR = ...`,
# but the assignment can be split across lines by a formatter, which a
# same-line grep would miss. Newlines are folded first so the check is robust
# to formatting rather than to one particular shape.
#
# Searching the whole venus directory rather than a known file, so moving the
# assignment somewhere else does not evade the check.
wired=$(
  find "$VENUS" \( -name '*.c' -o -name '*.h' \) -print0 2>/dev/null \
    | xargs -0 cat 2>/dev/null \
    | tr '\n' ' ' \
    | grep -oE 'dispatch_vk[A-Za-z0-9]*Video[A-Za-z0-9]*[[:space:]]*=[[:space:]]*[A-Za-z_&][A-Za-z0-9_]*' \
    | grep -v '=[[:space:]]*NULL' || true
)

wired_count=$(printf '%s\n' "$wired" | grep -c . || true)

# Condition 2: the thirteen decode commands must all be wired. Counting rather
# than naming each one, because the set is fixed by the three extensions and a
# name list here would be another hand-written set to drift.
if [ "$wired_count" -lt 13 ]; then
  echo "exposure-gate: FAIL -- only $wired_count video dispatch entries wired, expected 13" >&2
  printf '%s\n' "$wired" | sort -u | sed 's/^/  /' >&2
  status=1
else
  echo "exposure-gate: $wired_count video dispatch entries wired"
fi

# Condition 3b: no ENCODE command dispatched. Carried over verbatim: encode is
# unimplemented, so a non-NULL entry would be a reachable decoder with nothing
# behind it.
wired_encode=$(printf '%s\n' "$wired" | grep -iE 'encode' || true)
if [ -n "$wired_encode" ]; then
  echo "exposure-gate: FAIL -- an ENCODE command has a dispatch entry:" >&2
  printf '%s\n' "$wired_encode" | sort -u | sed 's/^/  /' >&2
  status=1
else
  echo "exposure-gate: no video ENCODE command has a dispatch entry"
fi

if [ "$status" != 0 ]; then
  echo >&2
  echo "  The capset is read as a promise the renderer can execute the WHOLE" >&2
  echo "  extension. Advertising more than is wired is a reachable decoder" >&2
  echo "  with no implementation behind it; advertising less silently breaks" >&2
  echo "  decode. Encode is unimplemented and must stay absent from both." >&2
  exit 1
fi

echo "exposure-gate: PASS -- decode advertised and wired, encode absent"
