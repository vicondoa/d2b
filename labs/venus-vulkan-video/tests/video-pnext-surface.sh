#!/usr/bin/env bash
# Enumerate the NON-video commands whose pNext chains accept a video struct.
#
# This is the E5 surface: entry points a guest can reach today, with video
# unadvertised, that will decode a well-formed video pNext and pass it to the
# host driver. The generator emits
#
#     vk->{proc_create}(args->device, args->{create_info}, ...)
#
# so pCreateInfo reaches the host with its pNext chain intact.
#
# The list is derived from the generated renderer rather than written by hand,
# for the same reason the array-cap audit is: this surface was hand-listed
# twice during W2 planning and was incomplete both times. vkCreateQueryPool in
# particular was named by nobody and found only by enumerating.
#
# Prints the pNext dispatch functions belonging to non-video structs that
# contain a video sType case. With --check, compares against a committed golden
# list and fails if the surface grew.
#
# Usage: video-pnext-surface.sh [--check <golden>] [--snapshot <out>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "pnext-surface: $*" >&2; exit 1; }

MODE=list
ARG=""
case "${1:-}" in
  --check)    MODE=check;    ARG="${2:-}" ;;
  --snapshot) MODE=snapshot; ARG="${2:-}" ;;
  "")         ;;
  *)          die "unknown option $1" ;;
esac

[ -d "$VP_DIR" ] || die "no venus-protocol at $VP_DIR"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

"$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$out" >/dev/null 2>&1 \
  || die "renderer generation failed"

# A chain belongs to the video surface if its own struct is not itself a video
# struct but one of its cases accepts a video sType. Chains rooted at a video
# struct are only reachable from video commands, which are NULL-dispatched.
#
# Both halves of that test are driven by the derived sType list rather than by
# a name substring. The substring version was wrong twice: `VK_STRUCTURE_TYPE_VIDEO`
# missed VK_STRUCTURE_TYPE_QUEUE_FAMILY_VIDEO_PROPERTIES_KHR, and
# `vn_decode_VkVideo` missed VkPhysicalDeviceVideoFormatInfoKHR. Neither name
# has the token where the pattern expected it.
stypes=$(
  "$PYTHON" - "$VP_DIR/xmls/vk.xml" <<'PY'
import re, sys, pathlib
xml = pathlib.Path(sys.argv[1]).read_text()
EXTS = r'VK_KHR_video_(?:queue|decode_queue|decode_h264)'
out = set()
for m in re.finditer(r'<extension[^>]*name="(%s)"[^>]*>(.*?)</extension>' % EXTS,
                     xml, re.S):
    for e in re.finditer(r'<enum[^>]*name="(VK_STRUCTURE_TYPE_[A-Z0-9_]+)"', m.group(2)):
        out.add(e.group(1))
print("|".join(sorted(out)))
PY
) || die "sType derivation failed"
[ -n "$stypes" ] || die "derived an empty sType list"

# The struct a chain belongs to, as a regex over the same sType set: a chain
# named vn_decode_VkFooBar_pnext belongs to VkFooBar.
video_structs=$(
  "$PYTHON" - "$VP_DIR/xmls/vk.xml" <<'PY'
import re, sys, pathlib
xml = pathlib.Path(sys.argv[1]).read_text()
EXTS = r'VK_KHR_video_(?:queue|decode_queue|decode_h264)'
out = set()
for m in re.finditer(r'<extension[^>]*name="(%s)"[^>]*>(.*?)</extension>' % EXTS,
                     xml, re.S):
    for t in re.finditer(r'<type name="(Vk[A-Za-z0-9]+)"', m.group(2)):
        out.add(t.group(1))
print("|".join(f"vn_decode_{n}_pnext" for n in sorted(out)))
PY
) || die "video struct derivation failed"
[ -n "$video_structs" ] || die "derived an empty video struct list"

surface=$(
  for f in "$out"/*.h; do
    awk -v stypes="$stypes" '
      /^vn_decode_[A-Za-z0-9_]*_pnext[a-z_]*\(/ {
        fn = $0; sub(/\(.*/, "", fn); inb = 1; hit = 0; next
      }
      inb && $0 ~ stypes { hit = 1 }
      inb && /^}/ { if (hit) print fn; inb = 0 }
    ' "$f"
  done | grep -Ev "^($video_structs)" | sort -u
)

case $MODE in
  snapshot)
    [ -n "$ARG" ] || die "--snapshot needs an output path"
    printf '%s\n' "$surface" > "$ARG"
    echo "pnext-surface: wrote $(printf '%s\n' "$surface" | grep -c .) entries to $ARG"
    ;;
  check)
    [ -n "$ARG" ] && [ -f "$ARG" ] || die "--check needs an existing golden file"
    if diff <(printf '%s\n' "$surface") <(sort -u "$ARG") > /tmp/pnext.diff 2>&1; then
      echo "pnext-surface: PASS -- $(printf '%s\n' "$surface" | grep -c .) entry points, unchanged"
      printf '%s\n' "$surface" | sed 's/^/  /'
    else
      echo "pnext-surface: FAIL -- the E5 surface changed:" >&2
      sed 's/^/  /' /tmp/pnext.diff >&2
      echo >&2
      echo "  Each of these decodes a guest-supplied video struct and passes it" >&2
      echo "  to the host driver. A new one is a new entry point that E5 must" >&2
      echo "  reject, and a removed one means a test is now dead. Update the" >&2
      echo "  golden only together with the E5 handling and its test." >&2
      exit 1
    fi
    ;;
  list)
    printf '%s\n' "$surface"
    ;;
esac
