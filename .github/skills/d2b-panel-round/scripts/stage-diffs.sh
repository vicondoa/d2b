#!/usr/bin/env bash
# Stage byte-identical review evidence for one panel round.
#
#   stage-diffs.sh <base> <prev-tip> <round-id> --selection <selection.json>
#                  [--lifecycle <lifecycle-id>] [--discovery-request PATH]
#                  [--ledger PATH] [--responses PATH] [--self-verification PATH]
#                  [--verification-dir PATH] [--approval PATH]
#
# <base>      branch base commit or ref
# <prev-tip>  commit the previous round reviewed; pass <base> for round 1
# <round-id>  qualified round address, e.g. spec001w1-r2
#
# Panel reviewers have no shell. Everything they read is written here.
set -euo pipefail

if [ "$#" -lt 3 ]; then
  echo "usage: stage-diffs.sh <base> <prev-tip> <round-id> --selection <selection.json> [--lifecycle <lifecycle-id>]" >&2
  exit 2
fi

base="$1"
prev="$2"
round="$3"
shift 3

case "$round" in
  */*|..*|"") echo "refusing round id with a path separator: $round" >&2; exit 2 ;;
esac

lifecycle=""
selection_path=""
discovery_request_path=""
ledger_path=""
responses_path=""
self_verification_path=""
verification_dir=""
approval_path=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --lifecycle)
      [ "$#" -ge 2 ] || { echo "--lifecycle requires a value" >&2; exit 2; }
      lifecycle="$2"
      shift 2
      ;;
    --selection)
      [ "$#" -ge 2 ] || { echo "--selection requires a path" >&2; exit 2; }
      selection_path="$2"
      shift 2
      ;;
    --discovery-request)
      [ "$#" -ge 2 ] || { echo "--discovery-request requires a path" >&2; exit 2; }
      discovery_request_path="$2"
      shift 2
      ;;
    --ledger)
      [ "$#" -ge 2 ] || { echo "--ledger requires a path" >&2; exit 2; }
      ledger_path="$2"
      shift 2
      ;;
    --responses)
      [ "$#" -ge 2 ] || { echo "--responses requires a path" >&2; exit 2; }
      responses_path="$2"
      shift 2
      ;;
    --self-verification)
      [ "$#" -ge 2 ] || { echo "--self-verification requires a path" >&2; exit 2; }
      self_verification_path="$2"
      shift 2
      ;;
    --verification-dir)
      [ "$#" -ge 2 ] || { echo "--verification-dir requires a path" >&2; exit 2; }
      verification_dir="$2"
      shift 2
      ;;
    --approval)
      [ "$#" -ge 2 ] || { echo "--approval requires a path" >&2; exit 2; }
      approval_path="$2"
      shift 2
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 2
      ;;
  esac
done

if [ -z "$selection_path" ]; then
  echo "--selection is required; staging without the authoritative lifecycle selection is refused" >&2
  exit 2
fi

if [[ "$round" =~ ^([[:alnum:]]+)-r([1-9][0-9]*)$ ]]; then
  wave="${BASH_REMATCH[1]}"
  round_number=$((10#${BASH_REMATCH[2]}))
else
  echo "round id must end in -r<N>, for example spec001w1-r2: $round" >&2
  exit 2
fi

if [ -z "$lifecycle" ]; then
  lifecycle="$wave"
fi
case "$lifecycle" in
  */*|..*|"") echo "refusing lifecycle id with a path separator: $lifecycle" >&2; exit 2 ;;
esac
if [[ "$lifecycle" == *\"* || "$lifecycle" == *\\* || "$lifecycle" == *$'\n'* ]]; then
  echo "refusing lifecycle id with JSON control characters: $lifecycle" >&2
  exit 2
fi
if [[ "$selection_path" == *\"* || "$selection_path" == *\\* || "$selection_path" == *$'\n'* ]]; then
  echo "refusing selection path with JSON control characters: $selection_path" >&2
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
for (const key of [
  "round", "lifecycle_id", "base", "previous_tip", "tip",
  "phase", "selection_path", "selection_sha256",
]) {
  if (typeof value[key] !== "string" || value[key].length === 0) {
    console.error(`${path}: address.json is missing ${key}`);
    process.exit(1);
  }
}
process.stdout.write(
  [
    value.round, value.lifecycle_id, value.base, value.previous_tip, value.tip,
    value.phase, value.selection_path, value.selection_sha256,
  ].join("\t"),
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
  IFS=$'\t' read -r recorded_round recorded_lifecycle _ _ recorded_tip _recorded_phase recorded_selection recorded_selection_sha <<<"$previous_fields"
  if [ "$recorded_round" != "$previous_round" ]; then
    echo "$previous_address records round $recorded_round, expected $previous_round" >&2
    exit 2
  fi
  if [ "$recorded_lifecycle" != "$lifecycle" ]; then
    echo "$previous_address records lifecycle $recorded_lifecycle, expected $lifecycle" >&2
    exit 2
  fi
  if [ "$prev_sha" != "$recorded_tip" ]; then
    echo "incremental range does not start at the previous recorded tip" >&2
    echo "  previous round  $previous_round" >&2
    echo "  recorded tip    $recorded_tip" >&2
    echo "  supplied tip    $prev_sha" >&2
    exit 2
  fi
  if [ -z "$recorded_selection" ] || [ ! -f "$recorded_selection" ]; then
    echo "previous review does not record a readable lifecycle selection" >&2
    exit 2
  fi
  actual_recorded_selection_sha="$(sha256sum "$recorded_selection" | cut -d' ' -f1)"
  if [ "$actual_recorded_selection_sha" != "$recorded_selection_sha" ]; then
    echo "previous review selection bytes disagree with address.json" >&2
    exit 2
  fi
  previous_roster="$(
    node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { readSelection } = await import(pathToFileURL(process.argv[2]).href);
process.stdout.write(readSelection(process.argv[1]).roster.join(","));
' "$recorded_selection" "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"
  )"
fi

if [ ! -f "$selection_path" ]; then
  echo "missing lifecycle selection: $selection_path" >&2
  exit 2
fi

selection_meta="$(
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, candidatePath, range, helperPath, lifecycle] = process.argv.slice(1);
const {
  changedPathsFromGitRange,
  candidateFromSelection,
  readSelection,
  selectionDigest,
  validateSelectionAgainstTable,
  writeCreateOrCompare,
} = await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
if (selection.lifecycle_id !== lifecycle) {
  throw new Error(
    `selection lifecycle ${selection.lifecycle_id} disagrees with staging lifecycle ${lifecycle}`,
  );
}
validateSelectionAgainstTable(selection);
const actual = changedPathsFromGitRange(range);
const declared = selection.phase === "verification"
  ? selection.classification_inputs.fix_delta?.changed_paths ??
    selection.classification_inputs.changed_paths
  : selection.classification_inputs.changed_paths;
if (actual.join("\u0000") !== declared.join("\u0000")) {
  throw new Error(
    `selection changed paths do not match git range ${range}; ` +
    `declared [${declared.join(", ")}], actual [${actual.join(", ")}]`,
  );
}
writeCreateOrCompare(candidatePath, candidateFromSelection(selection));
process.stdout.write([
  selection.phase,
  selectionDigest(selectionPath),
  selection.roster.join(","),
].join("\t"));
' "$selection_path" "$out/candidate.json" "$prev_sha..$tip" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" "$lifecycle"
)" || {
  echo "selection validation or git-range derivation failed" >&2
  exit 2
}
IFS=$'\t' read -r phase selection_sha256 selected_roster <<<"$selection_meta"
IFS=',' read -r -a panel_seats <<<"$selected_roster"
if [ "${#panel_seats[@]}" -eq 0 ]; then
  echo "no panel seat agents found under $root/.github/agents" >&2
  exit 2
fi
for seat in "${panel_seats[@]}"; do
  if [ ! -f "$root/.github/agents/panel-$seat.agent.md" ]; then
    echo "selected roster seat has no panel agent: $seat" >&2
    exit 2
  fi
done

if [ "$round_number" -gt 1 ]; then
  for seat in "${panel_seats[@]}"; do
    case ",$previous_roster," in
      *,"$seat",*)
        prior_verdict="$previous_dir/verdicts/$seat.json"
        if [ ! -s "$prior_verdict" ]; then
          echo "missing previous verdict for incumbent seat $seat: $prior_verdict" >&2
          echo "later reviews must give every incumbent seat its own prior verdict to verify" >&2
          exit 2
        fi
        ;;
      *)
        # A newly triggered seat has no prior verdict. Its first verification
        # request carries the complete ledger and an explicit null prior.
        ;;
    esac
  done
fi

for path_value in "$discovery_request_path" "$ledger_path" "$responses_path" \
  "$self_verification_path" "$verification_dir" "$approval_path"; do
  if [[ "$path_value" == *\"* || "$path_value" == *\\* || "$path_value" == *$'\n'* ]]; then
    echo "artifact path contains JSON control characters: $path_value" >&2
    exit 2
  fi
done

if [ -f "$out/address.json" ]; then
  if ! existing_fields="$(read_address "$out/address.json")"; then
    exit 2
  fi
  IFS=$'\t' read -r existing_round existing_lifecycle existing_base existing_prev existing_tip existing_phase existing_selection existing_selection_sha <<<"$existing_fields"
  if [ "$existing_round" != "$round" ] ||
     [ "$existing_lifecycle" != "$lifecycle" ] ||
     [ "$existing_base" != "$base_sha" ] ||
     [ "$existing_prev" != "$prev_sha" ] ||
     [ "$existing_tip" != "$tip" ] ||
     [ "$existing_phase" != "$phase" ] ||
     [ "$existing_selection" != "$selection_path" ] ||
     [ "$existing_selection_sha" != "$selection_sha256" ]; then
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

publish_no_replace() {
  local tmp="$1"
  local dest="$2"
  if [ -e "$dest" ]; then
    if ! cmp -s "$tmp" "$dest"; then
      echo "conflicting generated bytes at $dest; refusing to overwrite" >&2
      rm -f -- "$tmp"
      return 1
    fi
    rm -f -- "$tmp"
    return 0
  fi
  if ! ln "$tmp" "$dest"; then
    if [ -e "$dest" ] && cmp -s "$tmp" "$dest"; then
      rm -f -- "$tmp"
      return 0
    fi
    rm -f -- "$tmp"
    return 1
  fi
  rm -f -- "$tmp"
}

stage() {
  local dest="$1"
  shift
  local tmp="$dest.$$.tmp"
  if ! "$@" > "$tmp"; then
    rm -f -- "$tmp"
    return 1
  fi
  publish_no_replace "$tmp" "$dest"
}

stage "$out/delta.diff" git --no-pager diff "$prev_sha..$tip"
stage "$out/full.diff" git --no-pager diff "$base_sha..$tip"
stage "$out/commits.txt" git --no-pager log --no-decorate --oneline "$base_sha..$tip"

delta_sha="$(sha256sum "$out/delta.diff" | cut -d' ' -f1)"
full_sha="$(sha256sum "$out/full.diff" | cut -d' ' -f1)"

node --input-type=module -e '
import { pathToFileURL } from "node:url";
const args = process.argv.slice(1);
const helperPath = args.pop();
const { writeCreateOrCompare } = await import(pathToFileURL(helperPath).href);
const [path, round, lifecycle, selectionPath, base, previousTip, tip, phase,
  selectionSha, deltaSha, fullSha] = args;
writeCreateOrCompare(path, {
  round,
  lifecycle_id: lifecycle,
  selection_path: selectionPath,
  selection_sha256: selectionSha,
  phase,
  base,
  previous_tip: previousTip,
  tip,
  delta_sha256: deltaSha,
  full_sha256: fullSha,
});
' "$out/address.json" "$round" "$lifecycle" "$selection_path" "$base_sha" \
  "$prev_sha" "$tip" "$phase" "$selection_sha256" "$delta_sha" "$full_sha" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"

if [ ! -f "$out/evidence.md" ]; then
  evidence_tmp="$out/evidence.md.$$.tmp"
  cat > "$evidence_tmp" <<'MD'
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
  publish_no_replace "$evidence_tmp" "$out/evidence.md"
fi

for seat in "${panel_seats[@]}"; do
  note="$out/reviewer-notes/$seat.md"
  if [ -f "$note" ]; then
    continue
  fi
  note_tmp="$note.$$.tmp"
  cat > "$note_tmp" <<MD
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
  publish_no_replace "$note_tmp" "$note"
done

if [ -z "$discovery_request_path" ]; then
  discovery_request_path="$out/discovery-request.json"
fi
if [ -z "$ledger_path" ]; then
  ledger_path="$out/discovery-ledger.json"
fi
if [ -z "$responses_path" ]; then
  responses_path="$out/responses.json"
fi
if [ -z "$self_verification_path" ]; then
  self_verification_path="$out/self-verification.json"
fi
if [ -z "$verification_dir" ]; then
  verification_dir="$out/verification"
fi
if [ -z "$approval_path" ]; then
  approval_path="$out/approval.json"
fi

request_tmp="$out/review-request.md.$$.tmp"
dispatch_tmp="$out/dispatch-prompt.txt.$$.tmp"
trap 'rm -f -- "$request_tmp" "$dispatch_tmp"' EXIT
cat > "$request_tmp" <<MD
# Panel review request

This is the complete shared request for \`$round\` in lifecycle \`$lifecycle\`. Read the artifacts below
with \`view\`; do not substitute a prose summary for them.

## Review address

- Delta to review: \`$out/delta.diff\`
- Delta range: \`$prev_sha..$tip\`
- Full branch context: \`$out/full.diff\`
- Full range: \`$base_sha..$tip\`
- Phase: \`$phase\`
- Lifecycle selection: \`$selection_path\` (sha256 \`$selection_sha256\`)
- Staged candidate: \`$out/candidate.json\`
- Validation evidence and phase deliverable: \`$out/evidence.md\`
- Seat-specific notes: \`$out/reviewer-notes/<your-seat>.md\`
- Commit list: \`$out/commits.txt\`

## Generated lifecycle artifacts

The canonical generated artifacts for this phase are:

$(if [ "$phase" = "discovery" ]; then
  printf '%s\n' \
    "- Discovery request: \`$discovery_request_path\`" \
    "- Issue ledger: \`$ledger_path\`" \
    "- Implementation responses: \`$responses_path\`"
else
  printf '%s\n' \
    "- Immutable discovery ledger: \`$ledger_path\`" \
    "- Implementation responses: \`$responses_path\`" \
    "- Self-verification: \`$self_verification_path\`" \
    "- Verification requests: \`$verification_dir/<your-seat>.json\`" \
    "- Approval artifact: \`$approval_path\`"
fi)

## Required review behaviour

1. Read the full candidate in \`$out/full.diff\` in full. On discovery, this
   full candidate is the review target, not only the incremental delta. Report
   every reasonably discoverable actionable finding now; do not save
   observations for later rounds.
2. Read the incremental delta in \`$out/delta.diff\` as well. On verification,
   review it for resolution, regressions, and unsafe late BLOCKER or MAJOR
   findings without reopening comprehensive discovery.
3. Read the validation evidence and phase deliverable. Missing or insufficient
   coverage is a finding. Do not rerun validation unless your seat-specific
   notes explicitly ask you to.
4. Read your seat-specific notes. Judge any rebuttal on its merits.
5. Inspect the tree and the diff rather than trusting a summary of what was
   intended to change.
6. Confine findings to defects in the candidate or delta that would cause incorrect
   behaviour, mask a regression, or weaken a stated invariant. Put other
   observations in the summary.
7. Return exactly the JSON verdict required by your panel agent and no other
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

publish_no_replace "$request_tmp" "$out/review-request.md"

cat > "$dispatch_tmp" <<MD
Read and follow the complete phase-aware review request at $out/review-request.md. Use view to read every artifact it names, including the staged candidate, generated lifecycle artifacts, the delta and your seat-specific notes. Review the delta rather than a prose summary, and return only your seat's required JSON verdict.
MD
publish_no_replace "$dispatch_tmp" "$out/dispatch-prompt.txt"
trap - EXIT

echo "staged $out"
echo "  tip          $tip"
echo "  delta        $prev_sha..$tip  ($delta_sha)"
echo "  full         $base_sha..$tip  ($full_sha)"
echo
echo "Edit $out/evidence.md and any seat-specific notes before dispatching."
echo "Dispatch every seat with the exact contents of $out/dispatch-prompt.txt."
