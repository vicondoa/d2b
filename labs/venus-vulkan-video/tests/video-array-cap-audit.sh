#!/usr/bin/env bash
# Assert every video-reachable guest-controlled array allocation in the
# generated renderer is capped BEFORE the allocation.
#
# Why this is a gate and not a review step: the cap set has now been audited by
# hand twice and been wrong both times. The first pass covered only the H.264
# payload and missed pProfiles, pReferenceSlots and pBindSessionMemoryInfos.
# The second pass added those and still missed pVideoFormatProperties and
# pMemoryRequirements, because those are *output* arrays -- the guest supplies
# the capacity it wants filled, so the count is guest-controlled even though
# the data flows the other way, and they do not look like attack surface.
#
# Enumerating the generated code removes the judgement call. Every
# vn_cs_decoder_alloc_temp_array() reachable from a video command or video
# struct must have a cap in the lines immediately above it. A new video array
# with no entry in ARRAY_COUNT_LIMITS fails this gate on the commit that adds
# it, rather than at the next panel.

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { echo "cap-audit: $*" >&2; exit 1; }

[ -d "$VP_DIR" ] || die "no venus-protocol at $VP_DIR"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

"$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$out" >/dev/null 2>&1 \
  || die "renderer generation failed"

"$PYTHON" - "$out" <<'PY'
import pathlib, re, sys

d = pathlib.Path(sys.argv[1])
uncapped, capped = set(), set()

for p in sorted(d.glob("*.h")):
    lines = p.read_text().splitlines()
    fn = None
    for i, line in enumerate(lines):
        m = re.search(r'\b(vn_decode_\w+)\s*\(struct vn_cs_decoder', line)
        if m:
            fn = m.group(1)
        if 'vn_cs_decoder_alloc_temp_array' not in line or fn is None:
            continue
        f = re.search(r'(?:args|val)->(\w+) = vn_cs_decoder_alloc_temp_array', line)
        if not f:
            continue
        # The generator emits the cap immediately before the allocation, in the
        # same block.
        #
        # Matching only the `if (... > N) {` condition credited the SHAPE of a
        # cap without its EFFECT: replacing the guard body with a comment left
        # every array uncapped while this gate still reported all 10 as "capped
        # before allocation". Unbounded guest-controlled allocation, green gate.
        # So bind the condition to a body that actually stops decoding, and do
        # not enumerate accepted variable names -- any comparison against a
        # constant counts, provided it rejects.
        window = lines[max(0, i - 14):i]
        has_cap = False
        for j, wline in enumerate(window):
            if not re.search(r'if \(\s*\w+\s*>\s*\d+\s*\)\s*\{', wline):
                continue
            depth, body = 0, []
            for k in range(j, len(window)):
                depth += window[k].count('{') - window[k].count('}')
                body.append(window[k])
                if depth <= 0:
                    break
            btxt = "\n".join(body)
            if 'vn_cs_decoder_set_fatal' in btxt and re.search(r'\breturn\b', btxt):
                has_cap = True
                break
        (capped if has_cap else uncapped).add((fn, f.group(1)))

def video(s):
    return {(fn, f) for fn, f in s if 'ideo' in fn}

bad, good = video(uncapped), video(capped)

for fn, f in sorted(good):
    print(f"  capped    {f:<26} {fn}")

if bad:
    print()
    print("cap-audit: FAIL -- video-reachable arrays allocated without a cap:",
          file=sys.stderr)
    for fn, f in sorted(bad):
        print(f"  {f:<26} {fn}", file=sys.stderr)
    print(file=sys.stderr)
    print("  A guest-chosen count reaches vn_cs_decoder_alloc_temp_array() with",
          file=sys.stderr)
    print("  nothing bounding it. Add the array to ARRAY_COUNT_LIMITS in",
          file=sys.stderr)
    print("  vn_protocol.py with a limit grounded in the codec or the API, then",
          file=sys.stderr)
    print("  re-import the generated headers into the forks.", file=sys.stderr)
    sys.exit(1)

print()
print(f"cap-audit: PASS -- {len(good)} video-reachable arrays, all capped "
      f"before allocation")
PY
