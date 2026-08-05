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

if [[ "$round" =~ ^([[:alnum:]]+)-r([1-9][0-9]*)$ ]]; then
  wave="${BASH_REMATCH[1]}"
  round_number=$((10#${BASH_REMATCH[2]}))
else
  echo "round id must end in -r<N>, for example spec001w1-r2: $round" >&2
  exit 2
fi

root="$(git rev-parse --show-toplevel)"
cd "$root"

tip="$(git rev-parse HEAD)"
base_sha="$(git rev-parse "$base")"
prev_sha="$(git rev-parse "$prev")"

out="$root/.scratch/panel/$round"

read_address() {
  node - "$1" <<'NODE'
const fs = require("node:fs");
const path = process.argv[2];
let value;
try {
  value = JSON.parse(fs.readFileSync(path, "utf8"));
} catch (error) {
  console.error(`${path}: invalid address.json: ${error.message}`);
  process.exit(1);
}
for (const key of ["round", "base", "previous_tip", "tip"]) {
  if (typeof value[key] !== "string" || value[key].length === 0) {
    console.error(`${path}: address.json is missing ${key}`);
    process.exit(1);
  }
}
process.stdout.write(
  [value.round, value.base, value.previous_tip, value.tip].join("\t"),
);
NODE
}

previous_round=""
previous_dir=""
if [ "$round_number" -eq 1 ]; then
  if [ "$prev_sha" != "$base_sha" ]; then
    echo "round 1 must use the branch base as <prev-tip>" >&2
    echo "  base      $base_sha" >&2
    echo "  prev-tip  $prev_sha" >&2
    exit 2
  fi
else
  previous_round="$wave-r$((round_number - 1))"
  previous_dir="$root/.scratch/panel/$previous_round"
  previous_address="$previous_dir/address.json"
  if [ ! -f "$previous_address" ]; then
    echo "missing previous review address: $previous_address" >&2
    echo "stage reviews sequentially so the incremental range is derived from recorded evidence" >&2
    exit 2
  fi
  if ! previous_fields="$(read_address "$previous_address")"; then
    exit 2
  fi
  IFS=$'\t' read -r recorded_round _ _ recorded_tip <<<"$previous_fields"
  if [ "$recorded_round" != "$previous_round" ]; then
    echo "$previous_address records round $recorded_round, expected $previous_round" >&2
    exit 2
  fi
  if [ "$prev_sha" != "$recorded_tip" ]; then
    echo "incremental range does not start at the previous recorded tip" >&2
    echo "  previous round  $previous_round" >&2
    echo "  recorded tip    $recorded_tip" >&2
    echo "  supplied tip    $prev_sha" >&2
    exit 2
  fi
fi

mapfile -t panel_seats < <(
  find "$root/.github/agents" -maxdepth 1 -type f -name 'panel-*.agent.md' \
    -printf '%f\n' |
    sed -e 's/^panel-//' -e 's/\.agent\.md$//' |
    sort
)
if [ "${#panel_seats[@]}" -eq 0 ]; then
  echo "no panel seat agents found under $root/.github/agents" >&2
  exit 2
fi

if [ "$round_number" -gt 1 ]; then
  for seat in "${panel_seats[@]}"; do
    prior_verdict="$previous_dir/verdicts/$seat.json"
    if [ ! -s "$prior_verdict" ]; then
      echo "missing previous verdict for seat $seat: $prior_verdict" >&2
      echo "later reviews must give every seat its own prior verdict to verify" >&2
      exit 2
    fi
  done
fi

if [ -f "$out/address.json" ]; then
  if ! existing_fields="$(read_address "$out/address.json")"; then
    exit 2
  fi
  IFS=$'\t' read -r existing_round existing_base existing_prev existing_tip <<<"$existing_fields"
  if [ "$existing_round" != "$round" ] ||
     [ "$existing_base" != "$base_sha" ] ||
     [ "$existing_prev" != "$prev_sha" ] ||
     [ "$existing_tip" != "$tip" ]; then
    echo "review address $round was already staged for different commits" >&2
    echo "use the next review id instead of changing evidence beneath an existing address" >&2
    exit 2
  fi
fi

if [ -d "$out/verdicts" ] &&
   find "$out/verdicts" -maxdepth 1 -type f -name '*.json' -print -quit |
     grep -q .; then
  echo "review $round already has verdicts; use the next review id" >&2
  exit 2
fi

mkdir -p "$out/verdicts" "$out/reviewer-notes"

# Write-then-rename every artifact. A diff truncated by a signal or a full
# disk would otherwise sit at its final path, and a reviewer would read a
# partial delta as the whole change. The temp name carries the pid so two
# concurrent stagings cannot stomp each other, and a failed write is removed
# rather than left as residue - including a failed rename, which is the one
# path that would otherwise exit under `set -e` with the temp still there.
stage() {
  local dest="$1"
  shift
  local tmp="$dest.$$.tmp"
  if ! "$@" > "$tmp"; then
    rm -f -- "$tmp"
    return 1
  fi
  if ! mv -f "$tmp" "$dest"; then
    rm -f -- "$tmp"
    return 1
  fi
}

stage "$out/delta.diff" git --no-pager diff "$prev_sha..$tip"
stage "$out/full.diff" git --no-pager diff "$base_sha..$tip"
stage "$out/commits.txt" git --no-pager log --no-decorate --oneline "$base_sha..$tip"

delta_sha="$(sha256sum "$out/delta.diff" | cut -d' ' -f1)"
full_sha="$(sha256sum "$out/full.diff" | cut -d' ' -f1)"

addr_tmp="$out/address.json.$$.tmp"
trap 'rm -f -- "$addr_tmp"' EXIT
cat > "$addr_tmp" <<JSON
{
  "round": "$round",
  "base": "$base_sha",
  "previous_tip": "$prev_sha",
  "tip": "$tip",
  "delta_sha256": "$delta_sha",
  "full_sha256": "$full_sha"
}
JSON
mv -f "$addr_tmp" "$out/address.json"
trap - EXIT

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

for seat in "${panel_seats[@]}"; do
  note="$out/reviewer-notes/$seat.md"
  if [ -f "$note" ]; then
    continue
  fi
  cat > "$note" <<MD
# Reviewer notes for $seat

## Integrator rebuttals

None.

If a prior finding is disputed, replace "None." with the rebuttal and its
evidence. The reviewer may withdraw an incorrect finding and is not required
to withdraw a correct one.

## Reviewer-specific validation request

None. Reviewers do not rerun tests, builds, evals, exploits, or other long
validations unless this section explicitly asks this seat to do so.
MD
done

request_tmp="$out/review-request.md.$$.tmp"
dispatch_tmp="$out/dispatch-prompt.txt.$$.tmp"
trap 'rm -f -- "$addr_tmp" "$request_tmp" "$dispatch_tmp"' EXIT

cat > "$request_tmp" <<MD
# Panel review request

This is the complete shared request for \`$round\`. Read the artifacts below
with \`view\`; do not substitute a prose summary for them.

## Review address

- Delta to review: \`$out/delta.diff\`
- Delta range: \`$prev_sha..$tip\`
- Full branch context: \`$out/full.diff\`
- Full range: \`$base_sha..$tip\`
- Validation evidence and phase deliverable: \`$out/evidence.md\`
- Seat-specific notes: \`$out/reviewer-notes/<your-seat>.md\`
- Commit list: \`$out/commits.txt\`

## Required review behaviour

1. Read the delta in full. The delta is the review target; the full diff is
   context only.
2. Read the validation evidence and phase deliverable. Missing or insufficient
   coverage is a finding. Do not rerun validation unless your seat-specific
   notes explicitly ask you to.
3. Read your seat-specific notes. Judge any rebuttal on its merits.
4. Inspect the tree and the diff rather than trusting a summary of what was
   intended to change.
5. Confine findings to defects in the delta that would cause incorrect
   behaviour, mask a regression, or weaken a stated invariant. Put other
   observations in the summary.
6. Return exactly the JSON verdict required by your panel agent and no other
   text. \`signoff\` is true if and only if \`recommendations\` is empty.
MD

if [ "$round_number" -gt 1 ]; then
  cat >> "$request_tmp" <<MD

## Prior verdict obligation

- Your previous verdict: \`$previous_dir/verdicts/<your-seat>.json\`
- Previous reviewed tip: \`$prev_sha\`

Read your own previous verdict and verify every prior recommendation against
the current tree by inspection. Do not mark a finding resolved because the
integrator says it was fixed. Any content change invalidated every prior
sign-off, including a sign-off from a seat whose area appears unaffected.
MD
else
  cat >> "$request_tmp" <<'MD'

## Prior verdict obligation

This is the first review. There is no prior verdict to verify.
MD
fi

mv -f "$request_tmp" "$out/review-request.md"

cat > "$dispatch_tmp" <<MD
Read and follow the complete review request at $out/review-request.md. Use view to read every artifact it names, including the delta and your seat-specific notes. Review the delta rather than a prose summary, and return only your seat's required JSON verdict.
MD
mv -f "$dispatch_tmp" "$out/dispatch-prompt.txt"
trap - EXIT

echo "staged $out"
echo "  tip          $tip"
echo "  delta        $prev_sha..$tip  ($delta_sha)"
echo "  full         $base_sha..$tip  ($full_sha)"
echo
echo "Edit $out/evidence.md and any seat-specific notes before dispatching."
echo "Dispatch every seat with the exact contents of $out/dispatch-prompt.txt."
