#!/usr/bin/env bash
# Derive the complete video exposure surface from vk.xml.
#
# W2 planning found seven separate "doors" over four panel rounds, each by a
# different reviewer, each an individual instance:
#
#   1 guest enables an unadvertised extension via vkCreateDevice
#   2 NULL dispatch cannot be both closed and testable
#   3 videoCodecOperations via the queue-family pNext
#   4 direct video query command ids
#   5 video pNext structs on non-video commands
#   6 video enum/flag values in ordinary fields, inbound and outbound
#   7 video bits in base VkQueueFamilyProperties.queueFlags
#
# Doors 3, 5, 6 and 7 are the same thing seen four times: a video-tagged VALUE
# crossing to or from the host through a command that predates video. Writing a
# rule per door is how the list kept growing -- three hand-audits, three
# incomplete answers. So the surface is derived instead.
#
# Every enum value contributed by the three video extensions is enumerated from
# vk.xml and bucketed by where it can appear. That bucket list, not a prose
# rule, is what W2's rejection and scrubbing code must cover, and what its tests
# must enumerate over.
#
# Usage: video-value-surface.sh [--check <golden>] [--snapshot <out>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "value-surface: $*" >&2; exit 1; }

MODE=list
ARG=""
case "${1:-}" in
  --check)    MODE=check;    ARG="${2:-}" ;;
  --snapshot) MODE=snapshot; ARG="${2:-}" ;;
  "")         ;;
  *)          die "unknown option $1" ;;
esac

[ -f "$VP_DIR/xmls/vk.xml" ] || die "no vk.xml under $VP_DIR"

# Direction is derived from the GENERATED renderer, not guessed from the value
# name. A name-keyed model gave every value one direction, which is wrong for
# most of them: VkVideoCodecOperationFlagBitsKHR is decoded from guest video
# profiles AND encoded back through VkQueueFamilyVideoPropertiesKHR. Scanning
# for vn_decode_<Type> and vn_encode_<Type> answers which way each type
# actually travels.
gen=$(mktemp -d)
trap 'rm -rf "$gen"' EXIT
"$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$gen" >/dev/null 2>&1 \
  || die "renderer generation failed"

surface=$("$PYTHON" - "$VP_DIR/xmls/vk.xml" <<'PY'
import re, sys, pathlib

xml = pathlib.Path(sys.argv[1]).read_text()

# Only the three extensions this prototype adds.
EXTS = r'VK_KHR_video_(?:queue|decode_queue|decode_h264)'

# Everything the video extensions require: types AND directly-declared enum
# values. Deriving from required TYPES rather than from names is the point --
# VkQueryResultStatusKHR carries no "Video" anywhere in its name, and an
# earlier name-substring version of this script missed 159 of 218 values
# because of exactly that.
req_types, ext_values, extends = set(), set(), {}
for m in re.finditer(r'<extension[^>]*name="([A-Za-z0-9_]+)"[^>]*>(.*?)</extension>',
                     xml, re.S):
    ext, body = m.group(1), m.group(2)
    if re.fullmatch(EXTS, ext):
        for t in re.finditer(r'<type name="([A-Za-z0-9_]+)"', body):
            req_types.add(t.group(1))
        for e in re.finditer(r'<enum([^>]*)name="(VK_[A-Z0-9_]+)"', body):
            ext_values.add(e.group(2))
            x = re.search(r'extends="([A-Za-z0-9_]+)"', e.group(1))
            if x:
                extends[e.group(2)] = x.group(1)
        continue
    # Values contributed by OTHER extensions but gated on a video dependency.
    # VK_KHR_maintenance5 is advertised by this renderer and adds
    # VK_BUFFER_USAGE_2_VIDEO_DECODE_*_BIT_KHR under
    # depends="VK_KHR_video_decode_queue"; VkBufferUsageFlags2CreateInfo is
    # decoded on the ordinary vkCreateBuffer path, so those bits are reachable
    # through an advertised extension today.
    for r in re.finditer(r'<require([^>]*)>(.*?)</require>', body, re.S):
        if 'video' not in r.group(1):
            continue
        for e in re.finditer(r'<enum([^>]*)name="(VK_[A-Z0-9_]+)"', r.group(2)):
            ext_values.add(e.group(2))
            x = re.search(r'extends="([A-Za-z0-9_]+)"', e.group(1))
            if x:
                extends[e.group(2)] = x.group(1)

# Every value of every enum/bitmask type the extensions require.
type_values = {}
for m in re.finditer(r'<enums name="([A-Za-z0-9_]+)"[^>]*>(.*?)</enums>', xml, re.S):
    if m.group(1) not in req_types:
        continue
    for e in re.finditer(r'<enum[^>]*name="(VK_[A-Z0-9_]+)"', m.group(2)):
        type_values[e.group(1)] = m.group(1)

names = set(ext_values) | set(type_values)

def bucket(n):
    # Most specific first. Classification drives what W2 owes each value, so an
    # unclassifiable value is a hard failure rather than a silent default.
    if n.endswith('_EXTENSION_NAME') or n.endswith('_SPEC_VERSION'):
        return 'ext-metadata'
    if n.startswith('VK_ERROR_'):               return 'result-code'
    if n.startswith('VK_STRUCTURE_TYPE'):       return 'stype'
    if n.startswith('VK_IMAGE_LAYOUT'):         return 'image-layout'
    if n.startswith('VK_QUEUE_'):               return 'queue-flag'
    if n.startswith('VK_QUERY_TYPE'):           return 'query-type'
    if n.startswith('VK_QUERY_RESULT_STATUS'):  return 'query-result-status'
    if n.startswith('VK_QUERY_RESULT'):         return 'query-result-flag'
    if 'FORMAT_FEATURE' in n:                   return 'format-feature'
    if 'PIPELINE_STAGE' in n or 'ACCESS_' in n: return 'sync-bit'
    if '_USAGE_' in n and 'VIDEO' in n:         return 'usage-bit'
    # Encode values reached only by following a video dependency out of an
    # encode extension. This renderer advertises no encode extension, and E1
    # refuses unadvertised ones, so they are doubly unreachable. Classified
    # rather than filtered out, so that advertising encode later fails this
    # gate instead of silently widening the surface.
    if 'VIDEO_ENCODE' in n:                     return 'encode-unreachable'
    if n.startswith('VK_VIDEO_SESSION_CREATE'):  return 'session-flag'
    if n.startswith('VK_VIDEO_CODEC_OPERATION'):  return 'codec-operation'
    if n.startswith('VK_OBJECT_TYPE'):          return 'object-type'
    if n.startswith('VK_DEBUG_REPORT'):         return 'debug'
    # Values reached through a required TYPE rather than named by the
    # extension: chroma subsampling, bit depth, codec operation, capability
    # flags, coding-control flags, session-create flags, H.264 picture layout.
    t = type_values.get(n)
    if t:
        if 'CodecOperation' in t:               return 'codec-operation'
        if 'ChromaSubsampling' in t or 'ComponentBitDepth' in t:
            return 'profile-enum'
        if 'Capability' in t:                   return 'capability-flag'
        if 'CodingControl' in t:                return 'coding-control-flag'
        if 'SessionCreate' in t or 'SessionParameters' in t:
            return 'session-flag'
        if 'PictureLayout' in t:                return 'picture-layout'
        if 'DecodeUsage' in t or 'DecodeFlag' in t or 'DecodeCapability' in t:
            return 'decode-flag'
        if 'QueryResultStatus' in t:            return 'query-result-status'
    return 'unclassified'

# Direction determines whether W2 must REJECT on the way in or SCRUB on the
# NO direction column here. Direction is a property of a carrying SITE, not of
# a value, and deriving it at value level was wrong three separate ways:
#
#   - one direction per value name    50 of 103 values travel both ways
#   - vn_decode_<T> / vn_encode_<T>   matches the generic helper DEFINITIONS
#     existence                       the generator emits for every type
#   - call sites of <T>               invisible for bitmask values, which ride
#                                     in members serialized as raw VkFlags
#
# tests/video-site-manifest.sh derives (struct, member, carrier, direction)
# from the generated bodies and is the authority. This gate answers only
# "which video values exist, and in which family" -- deliberately narrower,
# because a second, weaker answer to the same question is how a wrong one gets
# quoted as evidence.
rows = sorted((bucket(n), n) for n in names)

for b, n in rows:
    print(f"{b}\t{n}")

bad = [(b, n) for b, n in rows if b == 'unclassified']
if bad:
    print("UNCLASSIFIED VALUES -- classify before use:", file=sys.stderr)
    for b, n in bad:
        t = type_values.get(n) or extends.get(n) or '?'
        print(f"  {n}  (type {t})", file=sys.stderr)
    sys.exit(2)
PY
) || die "surface derivation failed (see unclassified values above)"

case $MODE in
  snapshot)
    [ -n "$ARG" ] || die "--snapshot needs an output path"
    printf '%s\n' "$surface" > "$ARG"
    echo "value-surface: wrote $(printf '%s\n' "$surface" | grep -c .) values to $ARG"
    ;;
  check)
    [ -n "$ARG" ] && [ -f "$ARG" ] || die "--check needs an existing golden file"
    if diff <(printf '%s\n' "$surface") "$ARG" > /tmp/valsurf.diff 2>&1; then
      echo "value-surface: PASS -- $(printf '%s\n' "$surface" | grep -c .) values, unchanged"
      printf '%s\n' "$surface" | cut -f1-2 | uniq -c \
        | awk '{printf "  %-16s %-9s %s\n", $2, $3, $1}'
    else
      echo "value-surface: FAIL -- the video value surface changed:" >&2
      sed 's/^/  /' /tmp/valsurf.diff >&2
      echo >&2
      echo "  Each inbound value is something a guest can put in an ordinary" >&2
      echo "  command; each outbound value is host capability that can leak" >&2
      echo "  back. A new one needs rejection or scrubbing plus a test before" >&2
      echo "  the golden moves." >&2
      exit 1
    fi
    ;;
  list)
    printf '%s\n' "$surface"
    ;;
esac
