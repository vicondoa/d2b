#!/usr/bin/env bash
# Emit the carrying-site manifest: which struct member, in which direction,
# can carry a video value across the boundary.
#
# Both the gpu and c reviewers specified this independently, and both were
# right that the value-level surface is not enough. Knowing that
# VK_BUFFER_USAGE_2_VIDEO_DECODE_SRC_BIT_KHR exists and is "inbound" does not
# tell an implementer that it arrives through
# VkBufferUsageFlags2CreateInfo::usage chained onto vkCreateBuffer. An
# implementation could reject VkBufferCreateInfo::usage, miss the usage2 pNext
# field, and leave the value gate green.
#
# Why type-level derivation could not produce this: a bitmask value is never
# serialized through a helper named after its FlagBits type. The generator
# emits
#
#     vn_encode_VkFlags(enc, &val->queueFlags);
#
# which names neither VkQueueFlagBits, nor VkQueueFlags, nor video. The member
# is the only place the connection survives, so the member is the unit.
#
# Direction per site, from the generated bodies:
#   inbound   the member appears in a vn_decode_<Struct>* body
#   outbound  the member appears in a vn_encode_<Struct>* body
#   both      both
#
# Usage: video-site-manifest.sh [--check <golden>] [--snapshot <out>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "site-manifest: $*" >&2; exit 1; }

MODE=list
ARG=""
case "${1:-}" in
  --check)    MODE=check;    ARG="${2:-}" ;;
  --snapshot) MODE=snapshot; ARG="${2:-}" ;;
  "")         ;;
  *)          die "unknown option $1" ;;
esac

[ -f "$VP_DIR/xmls/vk.xml" ] || die "no vk.xml under $VP_DIR"

gen=$(mktemp -d)
trap 'rm -rf "$gen"' EXIT
"$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$gen" >/dev/null 2>&1 \
  || die "renderer generation failed"

manifest=$("$PYTHON" - "$VP_DIR/xmls/vk.xml" "$gen" <<'PY'
import re, sys, pathlib

xml = pathlib.Path(sys.argv[1]).read_text()
gen = pathlib.Path(sys.argv[2])
src = "\n".join(p.read_text() for p in gen.glob('*.h'))

EXTS = r'VK_KHR_video_(?:queue|decode_queue|decode_h264)'

# Enum/bitmask types the video extensions require, plus the types whose values
# they contribute to (VK_QUEUE_VIDEO_DECODE_BIT_KHR extends VkQueueFlagBits,
# which the extensions do not "require" but do extend).
video_types = set()
for m in re.finditer(r'<extension[^>]*name="([A-Za-z0-9_]+)"[^>]*>(.*?)</extension>',
                     xml, re.S):
    ext, body = m.group(1), m.group(2)
    is_video = bool(re.fullmatch(EXTS, ext))
    for r in re.finditer(r'<require([^>]*)>(.*?)</require>', body, re.S):
        if not is_video and 'video' not in r.group(1):
            continue
        for e in re.finditer(r'<enum([^>]*)name="(VK_[A-Z0-9_]+)"', r.group(2)):
            x = re.search(r'extends="([A-Za-z0-9_]+)"', e.group(1))
            if x:
                video_types.add(x.group(1))
    if is_video:
        for t in re.finditer(r'<type name="([A-Za-z0-9_]+)"', body):
            video_types.add(t.group(1))

# Types the video extensions DEFINE, as opposed to merely extend. A defined
# type is video in its entirety; an extended type contributes only the values
# video adds to it.
video_defined_types = set()
for m in re.finditer(r'<extension[^>]*name="(%s)"[^>]*>(.*?)</extension>' % EXTS,
                     xml, re.S):
    for t in re.finditer(r'<type name="([A-Za-z0-9_]+)"', m.group(2)):
        video_defined_types.add(t.group(1))

# FlagBits -> Flags. Attribute order varies in vk.xml, so accept either.
carriers = set(video_types)
flags_of = {}
for m in re.finditer(
        r'<type ([^>]*category="bitmask"[^>]*)>typedef <type>VkFlags\d*</type> '
        r'<name>([A-Za-z0-9_]+)</name>', xml):
    b = re.search(r'(?:requires|bitvalues)="([A-Za-z0-9_]+)"', m.group(1))
    if b:
        flags_of[b.group(1)] = m.group(2)
        if b.group(1) in video_types:
            carriers.add(m.group(2))

# Struct types. A struct-valued member's obligation is its own rows, reached
# recursively, so it is not a value site itself.
struct_types = set(re.findall(
    r'<type category="struct" name="(Vk[A-Za-z0-9]+)"', xml))

# Plain C scalars that appear as members of video-introduced structs.
SCALARS = {'uint8_t', 'uint16_t', 'uint32_t', 'uint64_t', 'int8_t', 'int16_t',
           'int32_t', 'int64_t', 'size_t', 'float', 'char', 'void'}

# Vulkan handle types. A handle is an object reference, not a value, so it
# has no video value set and a different obligation.
handle_types = set(re.findall(
    r'<type category="handle"[^>]*>.*?<name>(Vk[A-Za-z0-9]+)</name>', xml, re.S))

# Every serializer name shape the generator actually emits, discovered rather
# than assumed. The bare form is the one an earlier attempt missed, which is
# why VkQueueFamilyProperties looked like it had no serializer at all.
SHAPES = ['', '_temp', '_self', '_self_temp', '_self_partial_temp',
          '_partial_temp', '_pnext', '_pnext_temp', '_pnext_partial_temp']

# Index every serializer body once. Searching the whole tree per struct per
# shape is O(structs x shapes x filesize) and takes minutes; one pass is
# instant and gives exactly the same answer.
BODIES = {}
# Two declaration shapes. Struct serializers put the return type on its own
# line, so the function name starts a line. Command serializers are declared
# `static inline void vn_decode_vkFoo_args_temp(...)` all on one line, so an
# anchored ^(vn_...) misses every one of them -- which is how the command-
# parameter pass silently produced nothing on its first run.
for m in re.finditer(
        r'^(?:static\s+inline\s+[A-Za-z_][A-Za-z0-9_ *]*?\s+)?'
        r'(vn_(?:en|de)code_[A-Za-z0-9_]+)\(.*?\n\{\n(.*?)\n\}',
        src, re.S | re.M):
    # Strip comments before indexing. The generator emits
    #     /* skip val->queueFlags */
    # for members it deliberately does NOT serialize, so a word-boundary match
    # on the member name reads a skip marker as proof the member is carried --
    # exactly backwards. VkQueueFamilyProperties::queueFlags is the case: its
    # only appearance in any decode body is inside that comment, and it was
    # being reported as inbound.
    body = re.sub(r'/\*.*?\*/', ' ', m.group(2), flags=re.S)
    body = re.sub(r'//[^\n]*', ' ', body)
    BODIES.setdefault(m.group(1), []).append(body)

def bodies(prefix, struct):
    out = []
    for s in SHAPES:
        out.extend(BODIES.get('%s_%s%s' % (prefix, struct, s), []))
    return out

rows = []
# The closing tag must be anchored to a line start. Members contain inline
# <type>...</type> tags, so a non-greedy (.*?)</type> terminates at the first
# member's type and truncates the struct body to nothing -- which is how this
# derivation silently produced zero sites on the first attempt.
#
# The body must also not be allowed to run past the next struct's opening tag.
# finditer resumes from the end of the previous match, so one over-long match
# silently skips every struct it swallowed: VkBufferCreateInfo disappeared
# entirely that way, sType row included, while VkImageCreateInfo survived.
for m in re.finditer(
        r'<type category="struct" name="(Vk[A-Za-z0-9]+)"'
        r'((?:(?!<type category="struct").)*?)\n\s*</type>',
        xml, re.S):
    struct = m.group(1)
    dec = bodies('vn_decode', struct)
    enc = bodies('vn_encode', struct)
    if not dec and not enc:
        continue
    # Every serialized member of a struct the video extensions INTRODUCED is
    # video surface, whatever its own type. VkQueueFamilyQueryResultStatusPropertiesKHR
    # carries its capability in a plain VkBool32, so a carrier-type filter does
    # not see it -- yet the renderer encodes it straight back to the guest and
    # it rides the VkQueueFamilyProperties2 pNext. The struct being video is
    # what makes the member video.
    struct_is_video = struct in video_types
    for mem in re.finditer(r'<member[^>]*>(.*?)</member>', m.group(2), re.S):
        t = re.search(r'<type>([A-Za-z0-9_]+)</type>', mem.group(1))
        n = re.search(r'<name>([A-Za-z0-9_]+)</name>', mem.group(1))
        if not t or not n:
            continue
        if not struct_is_video and t.group(1) not in carriers:
            continue
        # pNext is a chain pointer, not a value carrier. The chain it points at
        # is video-pnext-surface.sh's question, answered there precisely with
        # 5 entry points. Same reason sType is excluded: a second, vaguer
        # answer to a question another gate answers well is how a wrong answer
        # gets quoted as evidence.
        if n.group(1) == 'pNext':
            continue
        member = n.group(1)
        word = re.compile(r'\b%s\b' % re.escape(member))
        d = any(word.search(b) for b in dec)
        e = any(word.search(b) for b in enc)
        if not d and not e:
            continue
        # sType is excluded. VkStructureType is a carrier because the video
        # extensions add sTypes to it, so every struct in the registry matches
        # and 446 of 543 rows were VkApplicationInfo.sType and its like. That
        # is noise here: an sType only matters as a pNext case label, and
        # video-pnext-surface.sh already owns that question with a precise
        # answer of 5 entry points. Two gates answering the same question, one
        # of them badly, is what the value-level direction column was.
        if t.group(1) == 'VkStructureType':
            continue
        rows.append((struct, member, t.group(1),
                     'both' if (d and e) else 'inbound' if d else 'outbound'))

# Command parameters, not just struct members.
#
# The c reviewer found that walking only <type category="struct"> misses a
# whole class: a video value carried by a scalar COMMAND PARAMETER has no
# struct to belong to. VK_QUERY_RESULT_WITH_STATUS_BIT_KHR is the case --
# vkGetQueryPoolResults takes a VkQueryResultFlags `flags` parameter, and
# vkr_query_pool.c forwards args->flags straight to the host. That was door 8,
# and the manifest could not see it.
#
# Commands serialize through _args* on the way in and _reply* on the way out,
# which is a different naming scheme from struct members and needed its own
# lookup.
CMD_SHAPES_IN = ['_args', '_args_temp']
CMD_SHAPES_OUT = ['_reply']

for m in re.finditer(r'<command(?![^>]*\balias\b)[^>]*>(.*?)</command>', xml, re.S):
    body = m.group(1)
    nm = re.search(r'<proto>.*?<name>([A-Za-z0-9_]+)</name>.*?</proto>', body, re.S)
    if not nm:
        continue
    cmd = nm.group(1)

    dec = [b for s in CMD_SHAPES_IN for b in BODIES.get('vn_decode_%s%s' % (cmd, s), [])]
    enc = [b for s in CMD_SHAPES_OUT for b in BODIES.get('vn_encode_%s%s' % (cmd, s), [])]
    if not dec and not enc:
        continue

    for p in re.finditer(r'<param[^>]*>(.*?)</param>', body, re.S):
        t = re.search(r'<type>([A-Za-z0-9_]+)</type>', p.group(1))
        n = re.search(r'<name>([A-Za-z0-9_]+)</name>', p.group(1))
        if not t or not n or t.group(1) not in carriers:
            continue
        param = n.group(1)
        word = re.compile(r'\b%s\b' % re.escape(param))
        d = any(word.search(b) for b in dec)
        e = any(word.search(b) for b in enc)
        if not d and not e:
            continue
        rows.append((cmd, param, t.group(1),
                     'both' if (d and e) else 'inbound' if d else 'outbound'))

# Which video VALUES each carrier can transport.
#
# A row naming (struct, member, carrier, direction) says where video crosses,
# but not what. The test reviewer's point: with one case per row, a
# representative value can pass while another video value for the same carrier
# is unhandled -- one VkImageLayout video layout rejected while the other two
# are forwarded, one usage bit scrubbed while another leaks.
#
# So each row carries its value set, and E3 must exercise all of them: OR the
# bits together for a bitmask, iterate every value for an enum.
value_sets = {}
for m in re.finditer(r'<extension[^>]*name="([A-Za-z0-9_]+)"[^>]*>(.*?)</extension>',
                     xml, re.S):
    ext, body = m.group(1), m.group(2)
    is_video = bool(re.fullmatch(EXTS, ext))
    for r in re.finditer(r'<require([^>]*)>(.*?)</require>', body, re.S):
        if not is_video and 'video' not in r.group(1):
            continue
        for e in re.finditer(r'<enum([^>]*)name="(VK_[A-Z0-9_]+)"', r.group(2)):
            x = re.search(r'extends="([A-Za-z0-9_]+)"', e.group(1))
            if x:
                value_sets.setdefault(x.group(1), set()).add(e.group(2))

# Values declared on a video type directly (chroma subsampling, bit depth,
# codec operation, and the rest) belong to that type's own value set.
#
# Only for types the video extensions DEFINE. A type that video merely EXTENDS
# -- VkImageLayout gains three video layouts, VkQueueFlagBits gains one bit --
# keeps only those contributed values, gathered above from extends=. Pulling
# the whole enum block for an extended type would list VK_IMAGE_LAYOUT_GENERAL
# and its nine siblings as video values, and a test iterating that set would be
# asserting things about layouts that have nothing to do with video.
for m in re.finditer(r'<enums name="([A-Za-z0-9_]+)"[^>]*>(.*?)</enums>', xml, re.S):
    if m.group(1) not in video_defined_types:
        continue
    for e in re.finditer(r'<enum[^>]*name="(VK_[A-Z0-9_]+)"', m.group(2)):
        value_sets.setdefault(m.group(1), set()).add(e.group(1))

def values_for(carrier):
    """The video values a carrier transports, via its FlagBits type if any."""
    out = set(value_sets.get(carrier, ()))
    for bits, flags in flags_of.items():
        if flags == carrier:
            out |= value_sets.get(bits, set())
    return sorted(out)
# Which guest-reachable commands carry each struct.
#
# The virtualization reviewer's point: a (struct, member) row collapses every
# command path that carries it. VkQueueFamilyProperties.queueFlags is one row,
# but the generated protocol encodes it through BOTH
# vkGetPhysicalDeviceQueueFamilyProperties and ...Properties2. A per-row test
# can exercise one path and leave the other leaking.
#
# So each row names its paths, and E3 must cover every one. Enforcement via a
# shared per-struct helper is the natural implementation, but that is the
# implementer's choice; what the manifest owes is the list to check against.
CMD_ALL = ['_args', '_args_temp', '_reply']

# struct -> structs its own serializers reference, for transitive closure.
# One level is not enough: vkGetPhysicalDeviceQueueFamilyProperties2 encodes
# VkQueueFamilyProperties2, which encodes VkQueueFamilyProperties through its
# _self helper. A direct scan finds only the Properties path and misses
# Properties2 -- which is exactly the two-path case the reviewer named.
struct_refs = {}
for fn, bodies_ in BODIES.items():
    m = re.match(r'vn_(?:en|de)code_(Vk[A-Za-z0-9]+)(?:_[a-z_]+)?$', fn)
    if not m:
        continue
    for b in bodies_:
        struct_refs.setdefault(m.group(1), set()).update(
            re.findall(r'vn_(?:en|de)code_(Vk[A-Za-z0-9]+)', b))

def reachable(start):
    seen, stack = set(), [start]
    while stack:
        s = stack.pop()
        for nxt in struct_refs.get(s, ()):
            if nxt not in seen:
                seen.add(nxt)
                stack.append(nxt)
    return seen

paths_for_struct = {}
for fn in BODIES:
    m = re.match(r'vn_(?:en|de)code_(vk[A-Za-z0-9]+?)(%s)$'
                 % '|'.join(CMD_ALL), fn)
    if not m:
        continue
    cmd = m.group(1)
    direct = set()
    for b in BODIES[fn]:
        direct.update(re.findall(r'vn_(?:en|de)code_(Vk[A-Za-z0-9]+)', b))
    for s in set(direct) | {r for d in direct for r in reachable(d)}:
        paths_for_struct.setdefault(s, set()).add(cmd)
for st, mem, carrier, d in sorted(set(rows)):
    # Every guest-reachable command that carries this struct. A row without
    # them collapses multiple paths into one: VkQueueFamilyProperties is
    # encoded through both vkGetPhysicalDeviceQueueFamilyProperties and
    # ...Properties2, so a single per-row test can cover one and leave the
    # other leaking.
    paths = ",".join(sorted(paths_for_struct.get(st, ()))) or st

    vals = values_for(carrier)
    if vals:
        print("\t".join((st, mem, carrier, d, ",".join(vals), paths)))
        continue

    # No video value set. That is not an error for every carrier -- it means
    # the row's obligation is a different one, and saying which is the point.
    # An empty column here would read as "no obligation", which is exactly
    # wrong for a member of a video struct.
    if carrier in handle_types:
        # An object reference. Obligation is the dependency pin
        # (params->session, session->bound memory), covered by D1-D8.
        kind = 'handle'
    elif carrier in carriers and carrier.endswith('FlagsKHR'):
        # A video flags type the spec reserves for future use: the FlagBits
        # type exists but defines no values. Obligation is "must be zero" --
        # which is a real check, and a stronger one than any value list, since
        # every bit is invalid rather than just the video ones.
        kind = 'reserved-zero'
    elif carrier.startswith('StdVideo'):
        # A codec-standard enum, defined in the vk_video headers rather than
        # vk.xml. Its values are H.264 semantics, and W1's round-trip suite
        # already validates StdVideo serialization field by field.
        kind = 'stdvideo'
    elif carrier in struct_types:
        # Nested struct. Its members are their own rows; this row exists so
        # the path is visible, not because the member itself carries a value.
        kind = 'nested-struct'
    elif carrier in SCALARS or (carrier.startswith('Vk') and carrier not in carriers):
        # A plain scalar inside a video-introduced struct. Nothing to reject by
        # value; inbound needs bounds validation, outbound is host capability
        # and needs scrubbing like any other video field.
        kind = 'scalar'
    else:
        print(f"site-manifest: no video values derivable for carrier {carrier} "
              f"at {st}.{mem}", file=sys.stderr)
        sys.exit(2)
    print("\t".join((st, mem, carrier, d, kind, paths)))

if not rows:
    print("site-manifest: derived no carrying sites at all -- the serializer "
          "name shapes or the carrier set are wrong", file=sys.stderr)
    sys.exit(2)
PY
) || die "manifest derivation failed"

case $MODE in
  snapshot)
    [ -n "$ARG" ] || die "--snapshot needs an output path"
    printf '%s\n' "$manifest" > "$ARG"
    echo "site-manifest: wrote $(printf '%s\n' "$manifest" | grep -c .) sites to $ARG"
    ;;
  check)
    [ -n "$ARG" ] && [ -f "$ARG" ] || die "--check needs an existing golden file"
    if diff <(printf '%s\n' "$manifest") "$ARG" > /tmp/sitemanifest.diff 2>&1; then
      echo "site-manifest: PASS -- $(printf '%s\n' "$manifest" | grep -c .) carrying sites, unchanged"
      printf '%s\n' "$manifest" | awk -F'\t' '{print $4}' | sort | uniq -c \
        | awk '{printf "  %-9s %s\n", $2, $1}'
    else
      echo "site-manifest: FAIL -- the carrying-site set changed:" >&2
      sed 's/^/  /' /tmp/sitemanifest.diff >&2
      echo >&2
      echo "  Each row is a struct member that can carry a video value across" >&2
      echo "  the boundary. A new row needs rejection or scrubbing AND a test" >&2
      echo "  before the golden moves; a removed row means a test is now dead." >&2
      exit 1
    fi
    ;;
  list)
    printf '%s\n' "$manifest"
    ;;
esac
