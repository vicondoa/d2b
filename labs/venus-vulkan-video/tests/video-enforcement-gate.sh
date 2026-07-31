#!/usr/bin/env bash
# Report, per carrying site, whether the renderer actually enforces the
# obligation that site implies.
#
# The test reviewer's finding, and it is the right one: the site manifest
# proves CLASSIFICATION, not ENFORCEMENT. Knowing that
# VkQueueFamilyProperties::queueFlags is an outbound site does not mean
# anything scrubs it. A golden that lists 543 sites and an implementation that
# handles none of them are both consistent with a green manifest gate.
#
# So this gate reads the manifest and, for each row, looks for the enforcement
# the row demands:
#
#   inbound  a rejection of the video bits in that member before the host call
#   outbound a scrub of the video bits from that member in guest-visible replies
#   both     both
#
# Right now the expected answer is "none of them", because W2's implementation
# has not been written. That is the point: the gate must be able to say so, and
# be seen saying so, before it is trusted to say the opposite.
#
# --expect-unenforced N pins the count, so progress is visible and a
# regression -- enforcement quietly disappearing -- fails rather than passing
# as "still zero".
#
# Usage: video-enforcement-gate.sh [--expect-unenforced <n>]

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${VIRGL_DIR:-$STATE/virglrenderer}"
MANIFEST="${VIDEO_SITE_MANIFEST:-$HERE/video-site-manifest-golden.txt}"

die() { echo "enforcement-gate: $*" >&2; exit 1; }

EXPECT=""
case "${1:-}" in
  --expect-unenforced) EXPECT="${2:-}" ;;
  "")                  ;;
  *)                   die "unknown option $1" ;;
esac

[ -f "$MANIFEST" ] || die "no site manifest at $MANIFEST"
[ -d "$VIRGL_DIR/src/venus" ] || die "no src/venus in $VIRGL_DIR"

# Hand-written renderer sources only. The generated protocol headers are not
# where enforcement can live: they are regenerated from venus-protocol and any
# edit there would be overwritten and would also desync the driver.
src=$(
  find "$VIRGL_DIR/src/venus" -maxdepth 1 -name '*.c' -o -maxdepth 1 -name '*.h' \
    | sort | tr '\n' ' '
)
[ -n "$src" ] || die "no renderer sources found"

PYTHON="${VENUS_LAB_PYTHON:-python3}"

# The pairing check runs in Python rather than as shell greps over a 375 KB
# string. The shell version was correct in isolation and wrong in place, and
# rather than keep chasing the quoting, the logic moved somewhere it can be
# read.
report=$("$PYTHON" - "$MANIFEST" $src <<'PY'
import os, re, sys, pathlib

manifest = pathlib.Path(sys.argv[1]).read_text().splitlines()
raw = "\n".join(pathlib.Path(p).read_text() for p in sys.argv[2:])

# Strip comments BEFORE any analysis. The scrub header documents each helper
# with a comment naming the exact type and member it handles, and those
# comments were being credited as evidence that the scrub existed -- the gate
# was reading its own documentation. Worse, a comment parses as no function at
# all, so it took the "inline scrub, nothing to wire" branch and skipped every
# wiring check. This is the second time in this lab that comments-as-code has
# inverted a gate; the manifest derivation lost 214 sites to it.
def strip_comments(text):
    out, i, n = [], 0, len(text)
    while i < n:
        if text.startswith('/*', i):
            end = text.find('*/', i + 2)
            i = n if end < 0 else end + 2
        elif text.startswith('//', i):
            end = text.find('\n', i)
            i = n if end < 0 else end
        elif text[i] in '"\'':
            q, i = text[i], i + 1
            while i < n and text[i] != q:
                i += 2 if text[i] == '\\' else 1
            i += 1
        else:
            out.append(text[i])
            i += 1
    return ''.join(out)

src = strip_comments(raw)
video_aware = re.compile(r'video', re.I)

# Function definitions, extracted by brace counting.
#
# Splitting on `\n}\n` was wrong. A chunk begins after the PREVIOUS function's
# closing brace, so it can start deep inside another file's declarations --
# headers end structs with `};`, which is not a boundary. The first `{` in such
# a chunk then belongs to an unrelated struct, the signature parse yields a name
# from that struct, and real helpers never enter the call graph at all. Their
# callers then read as unenforced. That is what made the gate disagree with an
# isolated reproduction of its own logic: file ordering changed the chunk
# boundaries, so the same code gave different answers on the same data.
#
# Signature is kept with the body: a scrub helper names its type only there.
DEF_RE = re.compile(
    r'(?m)^(?:[A-Za-z_]\w*[ \t\*]+)*([A-Za-z_]\w*)[ \t]*\([^;{}]*\)[ \t]*\n?\{')


def extract_functions(text):
    out = []
    for m in DEF_RE.finditer(text):
        start = m.start()
        i = text.index('{', m.start(1))
        depth, n = 0, len(text)
        while i < n:
            if text[i] == '{':
                depth += 1
            elif text[i] == '}':
                depth -= 1
                if depth == 0:
                    break
            i += 1
        out.append((m.group(1), text[start:i + 1]))
    return out


_DEFS = extract_functions(src)
FUNCS = [body for _, body in _DEFS]
_NAME_OF = {id(body): name for name, body in _DEFS}
video_funcs = [f for f in FUNCS if video_aware.search(f)]

def scrubbed_in_walk(site, member):
    """Is this member scrubbed inside the outbound scrub walk itself?

    A SECOND obligation, distinct from "is the value rejected". Populated-list
    scrubbing and capacity-count rewriting are different duties, and one
    manifest row was standing for both: vkr_video_fix_layout_capacity calls the
    scrub on a SCRATCH probe, names the member, and never touches the guest's
    list -- so deleting the real scrub left the site credited.

    Modelling them separately is what security and c both identified as the
    fix. The scrub walk is exactly the vkr_video_scrub_* family; the capacity
    helper is deliberately not in it, so requiring the credit to come from
    inside the walk separates the two duties without inspecting expressions.
    """
    m = re.compile(r'\b%s\b' % re.escape(member))
    for chunk in FUNCS:
        name = _NAME_OF.get(id(chunk)) or ''
        if not name.startswith('vkr_video_scrub_'):
            continue
        # Scoped to the block handling THIS struct. Two different structs can
        # own members of the same name -- VkPhysicalDeviceHostImageCopyProperties
        # and VkPhysicalDeviceVulkan14Properties both have pCopySrcLayouts -- so
        # a member-name-only check let one struct's scrub cover for the other.
        # That is the cross-struct masking already fixed in the coverage gate
        # and not carried here until it produced a third failed attempt.
        for block in re.split(r'\n      case ', chunk):
            if not re.search(r'\b%s\b' % re.escape(site), block):
                continue
            for call in re.finditer(r'\bvkr_video_\w+\s*\(([^;]*)\)', block):
                if m.search(call.group(1)):
                    return True
            if re.search(r'->%s\b[^;\n]*(&=|=)' % re.escape(member), block):
                return True
    return False


def enforced_for_command(site, member, command):
    """Is (site, member) enforced on THIS specific command's path?

    Reachability from *any* dispatch entry is not enough. A manifest row names
    every command that can carry the value, and 48 of the 189 rows name more
    than one. Crediting a row because one of its paths is guarded reports the
    others as covered while they forward the value straight to the host --
    vkCmdPipelineBarrier2 was guarded while vkCmdSetEvent2 and vkCmdWaitEvents2
    were not, and the row read as enforced.
    """
    chunks = reachable_from_command(command)
    if not chunks:
        return False

    if ('vkr_video_reject_present_%s' % site) in reachable_names_from(command):
        return True

    # A command name never stands alone in the source: it is embedded in
    # `vkr_dispatch_vkCmdBlitImage` and `struct vn_command_vkCmdBlitImage`, and
    # `_` is a word character, so a leading \b can never match. Command rows
    # therefore anchor only on the trailing boundary. Struct names DO stand
    # alone as type names, so they keep both anchors and stay strict.
    is_command = site[:1].islower()
    s = re.compile((r'%s\b' if is_command else r'\b%s\b') % re.escape(site))
    m = re.compile(r'\b%s\b' % re.escape(member))
    for f in chunks:
        if not (video_aware.search(f) and s.search(f)):
            continue
        # The member must appear in a video call's ARGUMENTS, not merely
        # somewhere in the function body. Mentioning it was enough to credit a
        # site: vkr_video_fix_layout_capacity names pCopySrcLayouts but only
        # rewrites counts when the list pointer is NULL, so deleting the
        # populated-list scrub left the site credited while populated replies
        # leaked. Being named by code that does something else is not
        # enforcement.
        for call in re.finditer(r'\bvkr_video_\w+\s*\(([^;]*)\)', f):
            if m.search(call.group(1)):
                return True
        # Struct validators check their own members directly rather than
        # passing them to a call, so a self-assignment or comparison on the
        # member inside a video-aware validator still counts.
        if re.search(r'\bs->%s\b' % re.escape(member), f) or \
           re.search(r'->%s\s*(&=|=|\?)' % re.escape(member), f):
            return True
    return False


_from_command = {}

def reachable_from_command(command):
    """Chunks transitively reachable from one command's dispatch entry."""
    if command in _from_command:
        return _from_command[command]

    entry = None
    for f in FUNCS:
        if re.search(r'vkr_dispatch_%s\s*\(' % re.escape(command), f.split('{')[0]):
            entry = f
            break
    if entry is None:
        _from_command[command] = []
        return []

    defs = {}
    for f in FUNCS:
        n = helper_name(f)
        if n:
            defs.setdefault(n, f)

    chunks, names, frontier = [entry], set(), [entry]
    while frontier:
        chunk = frontier.pop()
        for n, body in defs.items():
            if n in names:
                continue
            if re.search(r'\b%s\s*\(' % re.escape(n), chunk):
                names.add(n)
                chunks.append(body)
                frontier.append(body)
    _from_command[command] = chunks
    _from_command[(command, 'names')] = names
    return chunks


def reachable_names_from(command):
    reachable_from_command(command)
    return _from_command.get((command, 'names'), set())


def helper_name(chunk):
    """Name of the function this chunk defines, or None if it is a dispatch entry.

    The name is taken from the extraction pass rather than re-parsed out of the
    chunk head. Every attempt to re-parse it was wrong in a different way:
    single-line-only matching missed helpers whose return type sits on its own
    line, and once chunks could start mid-file the head belonged to an
    unrelated declaration entirely.
    """
    name = _NAME_OF.get(id(chunk))
    if name is None:
        return None
    return None if name.startswith('vkr_dispatch_') else name


_dispatched = None

def dispatched_commands():
    """Commands that actually have a dispatch entry in the source.

    This replaces a name heuristic. Gating used to ask whether a command path
    had "Video" in its name, which is a guess about reachability dressed up as
    a rule, and deriving from a name has been wrong every time it was tried in
    this lab. Whether a command can be invoked at all is a fact the source
    answers directly: no vkr_dispatch_ entry means no way in.
    """
    global _dispatched
    if _dispatched is None:
        _dispatched = set(re.findall(r'vkr_dispatch_(\w+)', src))
    return _dispatched


_reachable = None

def reachable_from_dispatch():
    """Helpers transitively called from a vkr_dispatch_* entry point.

    "Called somewhere" is not enough. A scrub helper called only by its own
    sibling wrapper inside the header satisfies that while the real dispatch
    path calls neither -- deleting the .c call site left the count unchanged.
    Only reachability from an actual entry point means the scrub RUNS.
    """
    global _reachable
    if _reachable is not None:
        return _reachable

    defs = {}
    for f in FUNCS:
        n = helper_name(f)
        if n:
            defs.setdefault(n, f)

    frontier = [f for f in FUNCS if helper_name(f) is None]
    seen, _reachable = set(), set()
    while frontier:
        chunk = frontier.pop()
        for n, body in defs.items():
            if n in seen:
                continue
            if re.search(r'\b%s\s*\(' % re.escape(n), chunk):
                seen.add(n)
                _reachable.add(n)
                frontier.append(body)
    return _reachable

enforced, unenforced = [], []
enforced, unenforced, gated = [], [], []
for line in manifest:
    if not line.strip():
        continue
    cols = line.split('\t')
    if len(cols) < 4:
        continue
    site, member, carrier, direction = cols[0], cols[1], cols[2], cols[3]
    paths = cols[5].split(',') if len(cols) > 5 and cols[5] else []

    # A row is enforced only when EVERY dispatched command that can carry the
    # value is guarded. Crediting the row for one guarded path reported the
    # rest as covered while they forwarded the value untouched; 48 rows name
    # more than one path, so this is the common case, not the corner.
    live = [p for p in paths if p in dispatched_commands()]
    ok = bool(live) and all(enforced_for_command(site, member, p) for p in live)
    # Outbound sites carry a second, separate obligation: the scrub walk itself
    # must handle the member. Without it the capacity helper stood in.
    if ok and direction in ('outbound', 'both') and not scrubbed_in_walk(site, member):
        ok = False
    if ok:
        enforced.append((direction, site, member, carrier))
        continue

    # A site whose command paths are all undispatchable is unreachable, and
    # reporting it as unenforced overstates the work. This credit is
    # CONTINGENT on video-exposure-gate.sh holding -- the moment one of those
    # commands gains a dispatch entry, these stop being closed and need real
    # enforcement. The dependency is the point: it is recorded here rather
    # than left as an assumption.
    #
    # Direction does not change this. An outbound reply from a command that
    # cannot be invoked is never produced, so VkVideoCapabilitiesKHR members
    # are as closed as the inbound ones; an earlier restriction to inbound
    # sites reported them as open work that no scrub could ever reach.
    if paths and not any(p in dispatched_commands() for p in paths):
        gated.append((direction, site, member, carrier))
        continue

    unenforced.append((direction, site, member, carrier))

print(len(enforced))
print(len(unenforced))
print(len(gated))
# SHOW_ENFORCED lists what IS credited instead of what is missing. Reviewing
# the credited list is how the loose matcher's false credits were caught, so
# it stays as a first-class debug surface rather than a throwaway edit.
show = os.environ.get("SHOW_ENFORCED")
for d, s, m, c in (enforced if show else unenforced)[:12]:
    print(f"  {d}\t{s}.{m} ({c})")
PY
) || die "enforcement check failed"

enforced=$(printf '%s\n' "$report" | sed -n 1p)
unenforced=$(printf '%s\n' "$report" | sed -n 2p)
gated=$(printf '%s\n' "$report" | sed -n 3p)
missing=$(printf '%s\n' "$report" | tail -n +4)

total=$((enforced + unenforced + gated))
echo "enforcement-gate: $total sites -- $enforced enforced, $gated gated by NULL dispatch, $unenforced unenforced"
echo "enforcement-gate: the $gated gated sites are closed only while their commands stay undispatched"

if [ -n "$EXPECT" ]; then
  if [ "$unenforced" != "$EXPECT" ]; then
    echo "enforcement-gate: FAIL -- expected $EXPECT unenforced sites, found $unenforced" >&2
    echo >&2
    if [ "$unenforced" -gt "$EXPECT" ]; then
      echo "  Enforcement went backwards, or new carrying sites appeared without" >&2
      echo "  it. Either way something the manifest says must be handled is not." >&2
    else
      echo "  Enforcement was added. That is good -- lower the expected count in" >&2
      echo "  the same commit, so the next regression is still caught." >&2
    fi
    exit 1
  fi
  echo "enforcement-gate: PASS -- unenforced count matches the pinned $EXPECT"
fi

if [ "$unenforced" != 0 ]; then
  echo
  if [ -n "${SHOW_ENFORCED:-}" ]; then label=enforced; else label=unenforced; fi
  echo "$label sites (first 12):"
  printf '%s\n' "$missing"
fi

exit 0
