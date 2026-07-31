#!/usr/bin/env bash
# Stage byte-identical review evidence for one panel round.
#
#   stage-diffs.sh <base> <prev-tip> <round-id>
#
# <base>      branch base commit or ref
# <prev-tip>  commit the previous round reviewed; pass <base> for round 1
# <round-id>  qualified round address, e.g. spec001w1-r2
#
# Panel reviewers have no shell. Everything they read is written here.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: stage-diffs.sh <base> <prev-tip> <round-id>" >&2
  exit 2
fi

base="$1"
prev="$2"
round="$3"

case "$round" in
  */*|..*|"") echo "refusing round id with a path separator: $round" >&2; exit 2 ;;
esac

root="$(git rev-parse --show-toplevel)"
cd "$root"

tip="$(git rev-parse HEAD)"
base_sha="$(git rev-parse "$base")"
prev_sha="$(git rev-parse "$prev")"

out="$root/.scratch/panel/$round"
mkdir -p "$out/verdicts"

git --no-pager diff "$prev_sha..$tip" > "$out/delta.diff"
git --no-pager diff "$base_sha..$tip" > "$out/full.diff"
git --no-pager log --no-decorate --oneline "$base_sha..$tip" > "$out/commits.txt"

delta_sha="$(sha256sum "$out/delta.diff" | cut -d' ' -f1)"
full_sha="$(sha256sum "$out/full.diff" | cut -d' ' -f1)"

cat > "$out/address.json" <<JSON
{
  "round": "$round",
  "base": "$base_sha",
  "previous_tip": "$prev_sha",
  "tip": "$tip",
  "delta_sha256": "$delta_sha",
  "full_sha256": "$full_sha"
}
JSON

if [ ! -f "$out/evidence.md" ]; then
  cat > "$out/evidence.md" <<'MD'
# Validation evidence

Replace this file before dispatching. Reviewers are told to treat missing or
insufficient evidence as a finding, so an unedited template will fail the
round on purpose.

## Commands run

| Command | Result |
|---|---|
|  |  |

## What this evidence does not cover

State it plainly. A green `test-rust` does not cover the fixture-dependent
contract crate, and an advisory job's pass is not evidence at all.

## Deliverable for this phase

One or two sentences. Reviewers confine findings to defects in the delta that
would break this deliverable or mask a regression.
MD
fi

echo "staged $out"
echo "  tip          $tip"
echo "  delta        $prev_sha..$tip  ($delta_sha)"
echo "  full         $base_sha..$tip  ($full_sha)"
echo
echo "Edit $out/evidence.md before dispatching."
