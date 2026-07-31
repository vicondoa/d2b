#!/usr/bin/env bash
#
# A rejection must also return a safe reply.
#
# Every gate in this lab checks whether a video value is REJECTED. None checked
# what the rejection RETURNS, and that gap produced four defects across three
# panel rounds:
#
#   * zeroing a reply struct also zeroed its sType, which the generated encoder
#     asserts on, so a guard became a guest-triggerable host assert;
#   * writing `args->pCount = 0` nulled the reply pointer instead of the count;
#   * returning before the host fill left output payloads as whatever was in
#     reply storage, and the encoder serialised it -- a rejection that leaked;
#   * a capacity query reported an unfiltered count.
#
# The enforcement gate stayed green through all four. This gate closes that
# class: for every dispatch function that rejects a video value and returns
# early with a status, every output member the generated reply encoder
# serialises must be written on that path.
#
# Usage: reply-hygiene-gate.sh [--expect-unsanitized <n>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${VIRGL_DIR:-$STATE/virglrenderer}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "reply-hygiene: $*" >&2; exit 1; }

EXPECT=""
case "${1:-}" in
  --expect-unsanitized) EXPECT="${2:-}" ;;
  "")                   ;;
  *)                    die "unknown option $1" ;;
esac

[ -d "$VIRGL_DIR/src/venus" ] || die "no src/venus in $VIRGL_DIR"

report=$("$PYTHON" - "$VIRGL_DIR/src/venus" <<'PY'
import glob
import os
import re
import sys
import pathlib

venus = sys.argv[1]
def strip_comments(text):
    out, i, n = [], 0, len(text)
    while i < n:
        if text.startswith('/*', i):
            e = text.find('*/', i + 2)
            i = n if e < 0 else e + 2
        elif text.startswith('//', i):
            e = text.find('\n', i)
            i = n if e < 0 else e
        else:
            out.append(text[i])
            i += 1
    return ''.join(out)


# Comments are stripped before analysis. The reject block carries a comment
# reading "sType and pNext survive", and a substring test for pnext matched it,
# so removing the actual zeroing call left the gate green. That is the fourth
# time comments-as-code has inverted a check in this lab -- and the second time
# I have written a new gate without carrying the lesson into it.
src = strip_comments("\n".join(pathlib.Path(p).read_text()
                for p in sorted(glob.glob(os.path.join(venus, '*.c')))))
proto = "\n".join(pathlib.Path(p).read_text()
                  for p in sorted(glob.glob(os.path.join(venus, 'venus-protocol', '*.h'))))

def written_in(member, block):
    """A reject path must WRITE the output member, not merely name it.

    Naming was the original test, and a mutation replacing the zeroing memset
    with a bare read of the same member kept the gate at 0 while restoring a
    real disclosure. The gate's own comment on the pNext case already said
    naming is not enough; the member case one line above did not apply it.
    """
    m = re.escape(member)
    pats = [
        # memset/memcpy over the member (by value or through the pointer)
        r'mem(?:set|cpy)\s*\([^;]*\b%s\b' % m,
        # assignment to the member, its target, or a field beneath it,
        # excluding comparisons (==, !=, <=, >=)
        r'\b%s\b\s*(?:->\s*\w+\s*|\[[^\]]*\]\s*)*=(?!=)' % m,
        r'\*\s*args->%s\s*=(?!=)' % m,
        # handed to a zero/scrub helper, which writes through the pointer
        r'vkr_video_(?:zero|scrub)\w*\s*\([^;]*\b%s\b' % m,
    ]
    return any(re.search(pt, block) for pt in pats)


def encoded_leaves(member_type):
    """Fields the type's own encoder serialises, i.e. what can actually leak.

    The gate modelled one obligation per top-level output member. The reply
    encoder writes several leaves beneath it, so zeroing any one leaf credited
    all of them: deleting the memset over ->imageFormatProperties left the gate
    at 0 because the sibling pNext zeroer still named the parent member. Same
    class as the (site, member) collapse -- several obligations sharing a row.
    """
    m = re.search(
        r'vn_encode_%s_self\(struct vn_cs_encoder \*enc,[^)]*\)\s*\{(.*?)\n\}'
        % re.escape(member_type), proto, re.S)
    if not m:
        return set()
    return set(re.findall(r'\bval->(\w+)\b', m.group(1)))


# Output members are the ones the generated reply encoder actually serialises
# through a pointer. `ret` is a status, not a payload, and is always written.
outputs = {}
pnext_outputs = {}
member_types = {}
for m in re.finditer(
        r'vn_encode_(vk\w+)_reply\(struct vn_cs_encoder \*enc,[^)]*\)\s*\{(.*?)\n\}',
        proto, re.S):
    cmd, body = m.group(1), m.group(2)
    # Outputs are encoded through more than one shape. Deriving them from
    # vn_encode_simple_pointer alone missed vkGetQueryPoolResults, whose pData
    # goes out as a blob array -- the gate's own output-derivation was a
    # hand-chosen pattern, which is the defect it exists to catch.
    members = set(re.findall(r'vn_encode_simple_pointer\(enc, args->(\w+)\)', body))
    members |= set(re.findall(r'vn_encode_blob_array\(enc, args->(\w+)', body))
    members |= set(re.findall(r'vn_encode_array_size\(enc, args->\w+\);\s*\n\s*vn_encode_\w+\(enc, args->(\w+)', body))
    if members:
        outputs[cmd] = members

    # An output struct's pNext chain is encoded one level down, inside
    # vn_encode_<Type>(), not in the reply body -- so a pattern keyed on the
    # reply body cannot see it. Resolve the member to its TYPE, then ask
    # whether that type has a _pnext encoder. Three earlier attempts that
    # skipped this resolution step all reported zero.
    for member_type, member in re.findall(
            r'vn_encode_(Vk\w+)\(enc, args->(\w+)\)', body):
        member_types[(cmd, member)] = member_type
        m2 = re.search(
            r'vn_encode_%s_pnext\(struct vn_cs_encoder \*enc,[^)]*\)\s*\{(.*?)\n\}'
            % re.escape(member_type), proto, re.S)
        # A _pnext encoder that supports no structs always writes NULL, so
        # nothing can escape through it. Flagging its mere existence reported
        # vkGetPhysicalDeviceExternalBufferProperties, whose encoder body is
        # literally "no known/supported struct" -- a false positive that would
        # have had me guard a chain that cannot carry anything.
        if m2 and 'case VK_STRUCTURE_TYPE_' in m2.group(1):
            pnext_outputs.setdefault(cmd, set()).add(member)


# Dispatch function bodies, by brace counting.
funcs_all = {}
for m in re.finditer(r'\n(\w+)\s*\([^;{}]*\)\s*\n?\{', src):
    i = src.index('{', m.end(1))
    depth = 0
    while i < len(src):
        if src[i] == '{':
            depth += 1
        elif src[i] == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    funcs_all[m.group(1)] = src[m.start():i + 1]

funcs = {}
for m in re.finditer(r'\n(vkr_dispatch_\w+)\s*\([^;{}]*\)\s*\n?\{', src):
    name = m.group(1)
    i = src.index('{', m.end(1))
    depth = 0
    while i < len(src):
        if src[i] == '{':
            depth += 1
        elif src[i] == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    funcs[name] = src[m.start():i + 1]

VIDEO = re.compile(r'vkr_video_(value|reject)_\w+\s*\(')

findings = []
for name, body in sorted(funcs.items()):
    cmd = name[len('vkr_dispatch_'):]
    outs = outputs.get(cmd)
    if not outs:
        continue

    # Rejection blocks: an if whose condition calls a video predicate and whose
    # body sets a status and returns.
    # A reject predicate need not be named vkr_video_*. The render-pass paths
    # call vkr_render_pass_has_video_layout, so keying on the name prefix
    # missed them -- the gate's own reject detection was a hand-chosen pattern,
    # which is the defect it exists to catch, for the second time inside the
    # gate itself. Any helper whose body calls a video predicate counts.
    reject_names = set(re.findall(r'vkr_video_\w+', src))
    for hname, hbody in funcs_all.items():
        if VIDEO.search(hbody):
            reject_names.add(hname)
    alt = '|'.join(sorted(re.escape(n) for n in reject_names))

    for blk in re.finditer(r'if \([^{]*?(?:%s)\([^{]*?\)\s*\{(.*?)\n   \}' % alt, body, re.S):
        block = blk.group(1)
        if 'return' not in block:
            continue
        if 'args->ret' not in block and 'set_fatal' in block:
            continue  # fatal path sends no reply
        for member in sorted(outs):
            if not written_in(member, block):
                findings.append((cmd, member))
                continue
            # A whole-struct zero of the member covers every leaf at once.
            if re.search(r'mem(?:set|cpy)\s*\(\s*&?\s*args->%s\s*,'
                         % re.escape(member), block):
                continue
            for leaf in sorted(encoded_leaves(member_types.get((cmd, member), ''))):
                if leaf == 'pNext':
                    continue  # covered by the dedicated pNext-chain obligation
                if not written_in(leaf, block):
                    findings.append((cmd, '%s->%s' % (member, leaf)))
        for member in sorted(pnext_outputs.get(cmd, ())):
            # The chain needs its own zeroing call; naming the member is not
            # enough, because zeroing the struct body leaves the chain intact.
            if not re.search(r'_pnext\s*\(', block):
                findings.append((cmd, '%s->pNext chain' % member))


seen, unique = set(), []
for f in findings:
    if f not in seen:
        seen.add(f)
        unique.append(f)

print(len(unique))
for cmd, member in unique:
    print("  %s leaves args->%s unwritten on the reject path" % (cmd, member))
PY
) || die "scan failed"

count=$(printf '%s\n' "$report" | sed -n 1p)
detail=$(printf '%s\n' "$report" | tail -n +2)

echo "reply-hygiene: $count rejected reply outputs left unwritten"

if [ -n "$EXPECT" ]; then
  if [ "$count" != "$EXPECT" ]; then
    echo "reply-hygiene: FAIL -- expected $EXPECT, found $count" >&2
    echo >&2
    printf '%s\n' "$detail" >&2
    echo >&2
    echo "  A rejection still produces a reply. An output the encoder" >&2
    echo "  serialises but the reject path never writes is whatever was" >&2
    echo "  left in reply storage." >&2
    exit 1
  fi
  echo "reply-hygiene: PASS -- matches the pinned $EXPECT"
fi

if [ "$count" != 0 ]; then
  echo
  printf '%s\n' "$detail"
fi

exit 0
