#!/usr/bin/env bash
#
# Dispatched commands whose args carry a video-capable type, but which the
# carrying-site manifest never lists.
#
# The manifest is derived entirely from vk.xml. virglrenderer also dispatches
# MESA vendor commands that vk.xml does not describe, so the manifest cannot
# list them and the enforcement gate cannot miss them -- there is no row to
# miss. vkCopyImageToMemoryMESA forwarded a guest image layout to the host that
# way, and every gate stayed green because the surface was invisible rather
# than unguarded.
#
# This gate closes the class. For every dispatched command absent from the
# manifest, it inspects the generated vn_command_<cmd> struct and reports any
# member whose TYPE is one the manifest treats as a video carrier.
#
# Usage: uncovered-dispatch-gate.sh [--expect-uncovered <n>]

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${VIRGL_DIR:-$STATE/virglrenderer}"
MANIFEST="${VIDEO_SITE_MANIFEST:-$HERE/video-site-manifest-golden.txt}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "uncovered-dispatch: $*" >&2; exit 1; }

EXPECT=""
case "${1:-}" in
  --expect-uncovered) EXPECT="${2:-}" ;;
  "")                 ;;
  *)                  die "unknown option $1" ;;
esac

[ -f "$MANIFEST" ] || die "no site manifest at $MANIFEST"
[ -d "$VIRGL_DIR/src/venus" ] || die "no src/venus in $VIRGL_DIR"

report=$("$PYTHON" - "$MANIFEST" "$VIRGL_DIR/src/venus" <<'PY'
import glob
import os
import re
import sys
import pathlib

manifest = pathlib.Path(sys.argv[1]).read_text().splitlines()
venus = sys.argv[2]

listed = set()
for line in manifest:
    cols = line.split('\t')
    if len(cols) < 6:
        continue
    listed.update(p for p in cols[5].split(',') if p)

# The carrier set is the types the generator actually emits a value check for,
# not every type the manifest mentions. Keying on the latter reported 226 pairs
# -- almost all of them commands that merely return VkResult -- which is noise
# that would bury the one real finding. A type is interesting here only if a
# guest can supply a video value in it and we know how to recognise one.
reject_header = pathlib.Path(os.path.join(venus, 'vkr_video_reject.h'))
carriers = set(re.findall(r'vkr_video_value_(\w+)\(', reject_header.read_text()))
carriers = {c for c in carriers if not re.match(r'^(Vk|Std)Video', c)}

src = "\n".join(pathlib.Path(p).read_text()
                for p in sorted(glob.glob(os.path.join(venus, '*.c'))))
dispatched = set(re.findall(r'vkr_dispatch_(\w+)\s*\(', src))

proto = "\n".join(pathlib.Path(p).read_text()
                  for p in sorted(glob.glob(os.path.join(venus, 'venus-protocol', '*.h'))))

# Generated command-args structs: struct vn_command_<cmd> { ... };
bodies = {}
for m in re.finditer(r'struct\s+vn_command_(\w+)\s*\{(.*?)\n\};', proto, re.S):
    bodies.setdefault(m.group(1), m.group(2))

# Vulkan struct bodies, so a carrier nested inside an args struct is still
# found. The MESA host-copy commands hold their image layout inside
# VkCopyImageToMemoryInfoMESA, not as a direct member of the args struct, so a
# direct-member check reported zero and missed the very case that motivated
# this gate.
structs = {}
for m in re.finditer(r'typedef struct (\w+)\s*\{(.*?)\n\}\s*\w+;', proto, re.S):
    structs.setdefault(m.group(1), m.group(2))

def members(body):
    return [ty for ty, _ in named_members(body)]


def named_members(body):
    """(type, name) pairs. The NAME matters for top-level command parameters:
    vkCmdCopyImage has srcImageLayout and dstImageLayout, both VkImageLayout,
    and collapsing them to one VkImageLayout obligation let deleting either
    guard pass while the other one covered for it."""
    return re.findall(r'\b(\w+)\b[\s\*]+(\w+)\s*(?:\[[^\]]*\])?\s*;', body)

def carries(body, seen, owner):
    """EVERY carrier type reachable from this struct, not the first one found.

    Returning on the first match let one carrier mask another in the same
    struct: VkPhysicalDeviceImageFormatInfo2 holds both VkImageUsageFlags and
    VkImageCreateFlags, so guarding usage alone credited the command and the
    create flags went unchecked with the gate green.
    """
    found = set()
    for typ, name in named_members(body):
        if typ in carriers:
            # Bound to the MEMBER, not just the owning struct. VkCopyImageInfo2
            # holds srcImageLayout and dstImageLayout, both VkImageLayout, so
            # an owner-only obligation let one member's check cover for the
            # other -- the same masking as top-level parameters, one level down.
            found.add((owner, name, typ))
        elif typ in structs and (typ, name) not in seen:
            seen.add((typ, name))
            sub = carries(structs[typ], seen, typ)
            found |= sub
            # Only a struct that actually carries a video value has an
            # occurrence obligation. Recording one for every nested struct
            # produced 179 findings, almost all on structs like VkExtent2D
            # that cannot carry anything.
            if not any(c != '@occurrence' for _, _, c in sub):
                continue
            # A command can reach one struct by several paths --
            # VkRenderingInfo reaches VkRenderingAttachmentInfo through
            # pColorAttachments, pDepthAttachment and pStencilAttachment.
            # Deduping by struct type collapsed them, so deleting one path's
            # validation left the other two covering for it.
            found.add((typ, name, '@occurrence'))
    return found

# Discovering a surface is not the same as guarding it. Counting uncovered
# pairs alone would hold at the pinned number with the guards deleted, which is
# the same existence-versus-enforcement mistake the enforcement gate already
# had to learn. Each discovered pair must also show a guard reachable from its
# own dispatch function.
# The call graph has to span the video headers too, not just the .c files.
# Every real guard bottoms out in a generated helper defined in
# vkr_video_reject.h, so a graph built from .c alone cannot reach the value
# check and reports plainly-guarded commands as open.
all_text = src
# vkr_video_validate.h joins these in W3, holding the allowlist guards.
for extra in ('vkr_video_reject.h', 'vkr_video_scrub.h', 'vkr_video_validate.h'):
    path = os.path.join(venus, extra)
    if os.path.exists(path):
        all_text += "\n" + pathlib.Path(path).read_text()

FUNCS = {}
for m in re.finditer(r'\n(\w+)\s*\([^;{}]*\)\s*\n?\{', all_text):
    start = m.start()
    i = all_text.index('{', m.start(1))
    depth = 0
    while i < len(all_text):
        if all_text[i] == '{':
            depth += 1
        elif all_text[i] == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    FUNCS[m.group(1)] = all_text[start:i + 1]

def reaches(entry, target):
    seen, frontier = set(), [entry]
    while frontier:
        chunk = frontier.pop()
        if re.search(r'\b%s\s*\(' % re.escape(target), chunk):
            return True
        for name, body in FUNCS.items():
            if name in seen or name.startswith('vkr_dispatch_'):
                continue
            if re.search(r'\b%s\s*\(' % re.escape(name), chunk):
                seen.add(name)
                frontier.append(body)
    return False


def checks_carrier(fn_name, carrier, seen=None):
    """Does this validator itself check the carrier, directly or via callees?"""
    if seen is None:
        seen = set()
    if fn_name in seen:
        return False
    seen.add(fn_name)
    body = FUNCS.get(fn_name)
    if not body:
        return False
    if re.search(r'vkr_video_value_%s\s*\(' % re.escape(carrier), body):
        return True
    for name in FUNCS:
        if name not in seen and re.search(r'\b%s\s*\(' % re.escape(name), body):
            if checks_carrier(name, carrier, seen):
                return True
    return False


def reachable_bodies(cmd):
    """Function bodies reachable from one command's dispatch entry."""
    entry = FUNCS.get('vkr_dispatch_' + cmd)
    if not entry:
        return []
    out, seen, frontier = [entry], set(), [entry]
    while frontier:
        chunk = frontier.pop()
        for name, body in FUNCS.items():
            if name in seen or name.startswith('vkr_dispatch_'):
                continue
            if re.search(r'\b%s\s*\(' % re.escape(name), chunk):
                seen.add(name)
                out.append(body)
                frontier.append(body)
    return out


def occurrence_guarded(cmd, owner, member):
    """Is this specific access path validated?

    Three guarding shapes exist in this renderer and all three are legitimate.
    Recognising only the first flagged seven plainly-guarded occurrences;
    loosening until the count reached zero would have accepted almost any
    nearby video call, which is the vacuity this gate exists to prevent. So
    each shape is matched explicitly:

      1. a generated validator applied to the occurrence
      2. a hand-written wrapper applied to the occurrence
      3. a loop binding the occurrence to a local, whose body then checks that
         local's members -- the call never names the occurrence at all
    """
    video_fns = [n for n in FUNCS
                 if n.startswith('vkr_video_') or re.search(r'video', n, re.I)]

    # Scoped to THIS command. A global scan by member name let commands cover
    # for each other: pColorAttachments appears in vkCreateRenderPass,
    # vkCreateRenderPass2 and vkCmdBeginRendering, so deleting one command's
    # guard was credited by another command's identically-named occurrence.
    for body in reachable_bodies(cmd):
        if not re.search(r'\b%s\b' % re.escape(member), body):
            continue

        # Shapes 1 and 2: some video-aware function is applied to an
        # expression naming this occurrence.
        for fn in video_fns:
            if re.search(r'\b%s\s*\([^;]*\b%s\b' % (re.escape(fn), re.escape(member)), body):
                return True

        # Shape 3: the occurrence is bound to a local, and a video-aware
        # function is applied to something derived from that local.
        for local in re.findall(
                r'\b(\w+)\s*=\s*&?[\w>.\-]*\b%s\b\s*\[' % re.escape(member), body):
            for fn in video_fns:
                if re.search(r'\b%s\s*\([^;]*\b%s\b' % (re.escape(fn), re.escape(local)), body):
                    return True
    return False


def guarded(cmd, carrier, owner=None, member=None):
    """Is the carrier checked anywhere reachable from this command's dispatch?

    A one-hop lookup is not enough. Real guards go through generated reject
    helpers -- vkr_video_reject_VkImageCreateInfo calls the value check for
    each carrier-typed member -- so a direct search for vkr_video_value_<T>
    in the dispatch body reported 29 commands unguarded that plainly are.
    Reachability has to be transitive, exactly as the enforcement gate learned.
    """
    entry = FUNCS.get('vkr_dispatch_' + cmd)
    if not entry:
        return False

    # A carrier found inside struct S is guarded only when S's OWN validator
    # checks it. Asking merely whether the check is reachable from the command
    # cannot distinguish which member it was applied to: removing the create
    # flags check from VkPhysicalDeviceImageFormatInfo2 left the same check
    # reachable through the pNext walker, so the mutation did not fire.
    # Binding the carrier to its owning struct is the dataflow fact that
    # reachability alone was missing.
    if carrier == '@occurrence':
        return occurrence_guarded(cmd, owner, member)

    if owner:
        # An inline check in the dispatch body is applied to THIS command's own
        # args, so it is as specific as a validator. The MESA host-copy structs
        # have no generated validator at all -- vk.xml does not describe them --
        # and are guarded exactly this way.
        if owner.startswith('args->'):
            # A parameter obligation needs the check applied to THAT parameter,
            # not merely to some parameter of the same type.
            return re.search(
                r'vkr_video_value_%s\s*\(\s*%s\b' % (re.escape(carrier), re.escape(owner)),
                entry) is not None
        if re.search(r'vkr_video_value_%s\s*\(' % re.escape(carrier), entry):
            return True

        # W3 allowlist and scrub guards.
        #
        # A guard can REJECT a disallowed value or ACCEPT only allowed ones or
        # MASK the disallowed bits off; all three close the surface, and after
        # W3 the last two are the common shapes because decode values are
        # supported rather than forbidden. Matching only the reject family was
        # a hand-written notion of what a guard looks like -- this wave's
        # recurring root cause, appearing inside the gate built to catch it.
        #
        # Bound to the MEMBER NAME, not merely to the function name. A first
        # attempt matched the function anywhere in the reachable text, which
        # credited the surface for the guard's DEFINITION in the header: with
        # the call deleted entirely the gate still reported zero unguarded.
        # That is credit-by-mention, the exact shape behind this program's
        # earlier false passes, and it was caught only by mutating the guard
        # away before pinning. A definition cannot satisfy the match below,
        # because its parameter is named `usage`/`props`, not the member.
        if member:
            for fn in (n for n in FUNCS if re.search(r'video', n, re.I)):
                for body in reachable_bodies(cmd):
                    if re.search(r'\b%s\s*\([^;]*\b%s\b'
                                 % (re.escape(fn), re.escape(member)), body):
                        return True
                    # The array form: the guard takes the array and the loop
                    # inside it reaches the member.
                    if re.search(r'\b%s_array\s*\(' % re.escape(fn), body) \
                       and re.search(r'\b%s\b' % re.escape(member),
                                     FUNCS.get(fn, '')):
                        return True

        validator = 'vkr_video_reject_' + owner
        if validator not in FUNCS:
            return False
        if member:
            # The owner validator must apply the check to THIS member.
            body = FUNCS.get(validator, '')
            if not re.search(r'vkr_video_value_%s\s*\(\s*s->%s\b'
                             % (re.escape(carrier), re.escape(member)), body):
                return False
        elif not checks_carrier(validator, carrier):
            return False
        return reaches(entry, validator)

    want = re.compile(r'vkr_video_value_%s\s*\(' % re.escape(carrier))
    seen, frontier = set(), [entry]
    while frontier:
        chunk = frontier.pop()
        if want.search(chunk):
            return True
        for name, body in FUNCS.items():
            if name in seen or name.startswith('vkr_dispatch_'):
                continue
            if re.search(r'\b%s\s*\(' % re.escape(name), chunk):
                seen.add(name)
                frontier.append(body)
    return False


# EVERY dispatched command, not only those the manifest omits. Restricting to
# manifest-absent commands meant a command the manifest listed for ONE parameter
# could carry a carrier type on ANOTHER with no guard and no gate noticing:
# vkGetPhysicalDeviceImageFormatProperties is listed for usage, and deleting the
# flags predicate left all four gates green. Manifest membership is not
# coverage; the args struct is what says what a command can carry.
findings, unguarded, report_lines = [], [], []
for cmd in sorted(dispatched):
    body = bodies.get(cmd)
    if not body:
        continue
    seen_types = set()
    for typ, name in named_members(body):
        if typ in carriers:
            # Top-level parameters are bound to their MEMBER NAME, so two
            # parameters of the same carrier type are two obligations.
            seen_types.add(('args->' + name, None, typ))
        elif typ in structs:
            seen_types |= carries(structs[typ], set(), typ)
    seen_types = {(o, m, c) if len(x) == 3 else x
                  for x in seen_types for (o, m, c) in [x if len(x) == 3 else (x[0], None, x[1])]}
    for owner, member, found in sorted(seen_types, key=lambda x: (x[0] or '', x[1] or '', x[2])):
        outside = cmd not in listed
        findings.append((cmd, found, outside))
        if not guarded(cmd, found, owner, member):
            label = ('%s.%s' % (owner, member or found)) if owner else found
            unguarded.append((cmd, label))
            report_lines.append("  %s carries %s [UNGUARDED]" % (cmd, label))

print(len([f for f in findings if f[2]]))
print(len(unguarded))
for line in report_lines:
    print(line)
PY
) || die "scan failed"

count=$(printf '%s\n' "$report" | sed -n 1p)
unguarded=$(printf '%s\n' "$report" | sed -n 2p)
detail=$(printf '%s\n' "$report" | tail -n +3)

echo "uncovered-dispatch: $count pairs outside the manifest, $unguarded unguarded"

if [ -n "$EXPECT" ]; then
  if [ "$count" != "$EXPECT" ]; then
    echo "uncovered-dispatch: FAIL -- expected $EXPECT, found $count" >&2
    echo >&2
    printf '%s\n' "$detail" >&2
    echo >&2
    echo "  A dispatched command carrying a video-capable type that the" >&2
    echo "  manifest does not list is a surface no other gate can see." >&2
    exit 1
  fi
  echo "uncovered-dispatch: PASS -- matches the pinned $EXPECT"
fi

# A surface this gate discovered but nothing guards is strictly worse than one
# it has not discovered: the pin says it is accounted for.
if [ "$unguarded" != 0 ]; then
  echo "uncovered-dispatch: FAIL -- $unguarded discovered surface(s) have no guard" >&2
  echo >&2
  printf '%s\n' "$detail" >&2
  exit 1
fi

if [ "$count" != 0 ]; then
  echo
  printf '%s\n' "$detail"
fi

exit 0
