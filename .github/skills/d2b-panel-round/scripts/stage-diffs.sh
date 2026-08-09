#!/usr/bin/env bash
# Stage byte-identical review evidence for one panel round.
#
#   stage-diffs.sh <base> <prev-tip> <round-id> --selection <selection.json>
#                  [--candidate <current-candidate.json>]
#                  [--lifecycle <lifecycle-id>] [--discovery-request PATH]
#                  --evidence PATH [--reviewer-notes-dir PATH]
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
  echo "usage: stage-diffs.sh <base> <prev-tip> <round-id> --selection <selection.json> --evidence PATH [--reviewer-notes-dir PATH] [--candidate <current-candidate.json>] [--lifecycle <lifecycle-id>] [--discovery-request PATH] [--ledger PATH] [--responses PATH] [--self-verification PATH] [--verification-dir PATH] [--approval PATH]" >&2
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
candidate_path=""
discovery_request_path=""
evidence_path=""
reviewer_notes_dir=""
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
    --candidate|--current-candidate)
      [ "$#" -ge 2 ] || { echo "$1 requires a path" >&2; exit 2; }
      if [ -n "$candidate_path" ] && [ "$candidate_path" != "$2" ]; then
        echo "--candidate and --current-candidate disagree" >&2
        exit 2
      fi
      candidate_path="$2"
      shift 2
      ;;
    --discovery-request)
      [ "$#" -ge 2 ] || { echo "--discovery-request requires a path" >&2; exit 2; }
      discovery_request_path="$2"
      shift 2
      ;;
    --evidence)
      [ "$#" -ge 2 ] || { echo "--evidence requires a path" >&2; exit 2; }
      evidence_path="$2"
      shift 2
      ;;
    --reviewer-notes-dir)
      [ "$#" -ge 2 ] || { echo "--reviewer-notes-dir requires a path" >&2; exit 2; }
      reviewer_notes_dir="$2"
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
if [ -z "$evidence_path" ]; then
  echo "--evidence is required; finalized validation evidence must be supplied before .complete" >&2
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
if [[ "$candidate_path" == *\"* || "$candidate_path" == *\\* || "$candidate_path" == *$'\n'* ]]; then
  echo "refusing candidate path with JSON control characters: $candidate_path" >&2
  exit 2
fi
for finalized_path in "$evidence_path" "$reviewer_notes_dir"; do
  if [[ "$finalized_path" == *\"* || "$finalized_path" == *\\* || "$finalized_path" == *$'\n'* ]]; then
    echo "refusing finalized review input path with JSON control characters: $finalized_path" >&2
    exit 2
  fi
done

root="$(git rev-parse --show-toplevel)"
cd "$root"
lifecycle_helper="$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"

tip="$(git rev-parse HEAD)"
base_sha="$(git rev-parse "$base")"
prev_sha="$(git rev-parse "$prev")"

panel_root="$root/.scratch/panel"
out="$panel_root/$round"
completion_marker="$out/.complete"
staged_selection_path="$out/selection.json"
staged_candidate_path="$out/current-candidate.json"
staged_discovery_request_path="$out/discovery-request.json"
staged_ledger_path="$out/discovery-ledger.json"
staged_responses_path="$out/responses.json"
staged_self_verification_path="$out/self-verification.json"
staged_approval_path="$out/approval.json"
staged_verification_dir="$out/verification"
staged_agent_definitions_dir="$out/agent-definitions"
display_panel_root="$panel_root"
display_out="$out"
display_completion_marker="$completion_marker"
display_staged_selection_path="$staged_selection_path"
display_staged_candidate_path="$staged_candidate_path"
display_staged_discovery_request_path="$staged_discovery_request_path"
display_staged_ledger_path="$staged_ledger_path"
display_staged_responses_path="$staged_responses_path"
display_staged_self_verification_path="$staged_self_verification_path"
display_staged_approval_path="$staged_approval_path"
display_staged_verification_dir="$staged_verification_dir"
display_staged_agent_definitions_dir="$staged_agent_definitions_dir"

# Staged packets remain exact and untruncated. Bound aggregate logical bytes
# across the entire packet root, including every lifecycle and incomplete
# packet. Operators may lower this ceiling for constrained environments, but
# may not raise the repository policy limit.
default_panel_root_max_bytes=$((1024 * 1024 * 1024))
panel_root_max_bytes="${D2B_PANEL_LIFECYCLE_MAX_BYTES:-$default_panel_root_max_bytes}"
if ! [[ "$panel_root_max_bytes" =~ ^[1-9][0-9]*$ ]] ||
   [ "$panel_root_max_bytes" -gt "$default_panel_root_max_bytes" ]; then
  echo "D2B_PANEL_LIFECYCLE_MAX_BYTES must be a positive integer no greater than $default_panel_root_max_bytes" >&2
  exit 2
fi

if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { ensureDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[2]).href);
ensureDirectoryNoFollow(process.argv[1]);
' "$display_panel_root" "$lifecycle_helper"; then
  echo "could not create or inspect the panel packet root" >&2
  exit 2
fi
if ! command -v flock >/dev/null 2>&1; then
  echo "flock is required for serialized panel packet root reservation" >&2
  exit 2
fi
if ! exec {panel_root_reservation_fd}<"$display_panel_root"; then
  echo "could not open the panel packet root for reservation" >&2
  exit 2
fi
if ! flock --exclusive "$panel_root_reservation_fd"; then
  echo "could not acquire the panel packet root reservation" >&2
  exit 2
fi
export D2B_PANEL_ROOT_FD="$panel_root_reservation_fd"
if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [panelRoot, descriptorText, helperPath] = process.argv.slice(1);
const { verifyDirectoryReservationNoFollow } =
  await import(pathToFileURL(helperPath).href);
verifyDirectoryReservationNoFollow(panelRoot, Number(descriptorText));
' "$display_panel_root" "$panel_root_reservation_fd" "$lifecycle_helper"; then
  echo "panel packet root reservation identity verification failed" >&2
  exit 2
fi

panel_root="/proc/self/fd/$panel_root_reservation_fd"
out="$panel_root/$round"
completion_marker="$out/.complete"
staged_selection_path="$out/selection.json"
staged_candidate_path="$out/current-candidate.json"
staged_discovery_request_path="$out/discovery-request.json"
staged_ledger_path="$out/discovery-ledger.json"
staged_responses_path="$out/responses.json"
staged_self_verification_path="$out/self-verification.json"
staged_approval_path="$out/approval.json"
staged_verification_dir="$out/verification"
staged_agent_definitions_dir="$out/agent-definitions"

bind_panel_root_path() {
  local value="$1"
  if [ -z "$value" ]; then
    printf '%s\n' "$value"
    return
  fi
  local absolute
  absolute="$(
    node --input-type=module -e '
import { resolve } from "node:path";
process.stdout.write(resolve(process.argv[1], process.argv[2]));
' "$root" "$value"
  )"
  case "$absolute" in
    "$display_panel_root")
      printf '%s\n' "$panel_root"
      ;;
    "$display_panel_root"/*)
      printf '%s/%s\n' "$panel_root" "${absolute#"$display_panel_root/"}"
      ;;
    *)
      printf '%s\n' "$value"
      ;;
  esac
}

verify_locked_panel_root() {
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { verifyDirectoryReservationNoFollow } =
  await import(pathToFileURL(process.argv[3]).href);
verifyDirectoryReservationNoFollow(process.argv[1], Number(process.argv[2]));
' "$display_panel_root" "$panel_root_reservation_fd" "$lifecycle_helper"
}

selection_path="$(bind_panel_root_path "$selection_path")"
candidate_path="$(bind_panel_root_path "$candidate_path")"
discovery_request_path="$(bind_panel_root_path "$discovery_request_path")"
evidence_path="$(bind_panel_root_path "$evidence_path")"
reviewer_notes_dir="$(bind_panel_root_path "$reviewer_notes_dir")"
ledger_path="$(bind_panel_root_path "$ledger_path")"
responses_path="$(bind_panel_root_path "$responses_path")"
self_verification_path="$(bind_panel_root_path "$self_verification_path")"
verification_dir="$(bind_panel_root_path "$verification_dir")"
approval_path="$(bind_panel_root_path "$approval_path")"

validate_bound_completion() {
  local marker="$1"
  node --input-type=module - "$marker" "$lifecycle_helper" <<'NODE'
import crypto from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
const markerPath = process.argv[2];
const helperPath = process.argv[3];
const { readBoundArtifactSetNoFollow } =
  await import(pathToFileURL(helperPath).href);
let marker;
let artifacts;
try {
  ({ marker, artifacts } = readBoundArtifactSetNoFollow(markerPath));
} catch (error) {
  console.error(`${markerPath}: invalid completion marker: ${error.message}`);
  process.exit(1);
}
const exactKeys = [
  "artifact_bytes",
  "artifact_kind",
  "artifact_sha256",
  "base",
  "complete",
  "delta_sha256",
  "full_sha256",
  "lifecycle_id",
  "phase",
  "previous_tip",
  "round",
  "schema_version",
  "selection_sha256",
  "tip",
].sort();
if (
  marker.artifact_kind !== "d2b-panel/stage-completion" ||
  marker.schema_version !== 2 ||
  marker.complete !== true ||
  !marker.artifact_sha256 ||
  !marker.artifact_bytes ||
  Object.keys(marker).sort().join("\0") !== exactKeys.join("\0")
) {
  console.error(
    `${markerPath}: completion marker is not a canonical byte-bound schema-version 2 marker`,
  );
  process.exit(1);
}
const digests = marker.artifact_sha256;
const sizes = marker.artifact_bytes;
if (
  !digests || Array.isArray(digests) || typeof digests !== "object" ||
  !sizes || Array.isArray(sizes) || typeof sizes !== "object" ||
  Object.keys(digests).sort().join("\0") !== Object.keys(sizes).sort().join("\0")
) {
  console.error(`${markerPath}: completion artifact maps disagree`);
  process.exit(1);
}
const root = path.dirname(markerPath);
for (const relative of Object.keys(digests).sort()) {
  if (
    relative === "" ||
    path.isAbsolute(relative) ||
    relative.split("/").includes("..") ||
    !/^[0-9a-f]{64}$/.test(digests[relative]) ||
    !Number.isSafeInteger(sizes[relative]) ||
    sizes[relative] < 0
  ) {
    console.error(`${markerPath}: invalid bound artifact entry ${relative}`);
    process.exit(1);
  }
  const bytes = artifacts[relative];
  const digest = crypto.createHash("sha256").update(bytes).digest("hex");
  if (digest !== digests[relative] || bytes.length !== sizes[relative]) {
    console.error(
      `${markerPath}: post-completion mutation of ${relative} is refused; ` +
      "its bytes disagree with the completion marker",
    );
    process.exit(1);
  }
}
NODE
}

secure_digest_size() {
  node --input-type=module - "$1" "$lifecycle_helper" <<'NODE'
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";
const { readFileNoFollow } =
  await import(pathToFileURL(process.argv[3]).href);
const bytes = readFileNoFollow(process.argv[2], { label: "staged artifact" });
process.stdout.write(
  `${crypto.createHash("sha256").update(bytes).digest("hex")}\t${bytes.length}`,
);
NODE
}

partial_cleanup_hint() {
  echo "partial pre-dispatch scratch directory is non-authoritative: $display_out" >&2
  echo "inspect its identity and contents before cleanup; do not recursively delete a path after an identity change" >&2
}

if [ -e "$out" ]; then
  if [ ! -f "$completion_marker" ]; then
    partial_cleanup_hint
    exit 2
  fi
  if ! validate_bound_completion "$completion_marker"; then
    echo "complete review $round failed canonical completion validation" >&2
    exit 2
  fi
fi

round_directory_owned=false
stage_exit() {
  local status="$?"
  if [ "$status" -ne 0 ] &&
     [ "$round_directory_owned" = true ] &&
     [ ! -f "$completion_marker" ]; then
    echo "preserved the incomplete staging directory; automatic pathname cleanup is refused" >&2
    partial_cleanup_hint
  fi
  exit "$status"
}
trap stage_exit EXIT

read_address() {
  node --input-type=module - "$1" "$lifecycle_helper" <<'NODE'
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";
const path = process.argv[2];
const { readFileNoFollow } =
  await import(pathToFileURL(process.argv[3]).href);
let value;
try {
  value = JSON.parse(readFileNoFollow(path, {
    encoding: "utf8",
    label: "review address",
  }));
} catch (error) {
  console.error(`${path}: invalid address.json: ${error.message}`);
  process.exit(1);
}
for (const key of ["round", "lifecycle_id", "base", "previous_tip", "tip"]) {
  if (typeof value[key] !== "string" || value[key].length === 0) {
    console.error(`${path}: address.json is missing ${key}`);
    process.exit(1);
  }
}
let phase = value.phase;
const selectionPath = value.selection_path ?? "";
let selectionSha256 = value.selection_sha256;
if (
  selectionPath &&
  (!phase || !selectionSha256)
) {
  let selectionBytes;
  try {
    selectionBytes = readFileNoFollow(selectionPath, {
      label: "recorded lifecycle selection",
    });
  } catch (error) {
    console.error(`${selectionPath}: unreadable lifecycle selection: ${error.message}`);
    process.exit(1);
  }
  let selection;
  try {
    selection = JSON.parse(selectionBytes);
  } catch (error) {
    console.error(`${selectionPath}: invalid lifecycle selection: ${error.message}`);
    process.exit(1);
  }
  phase ||= selection.phase;
  selectionSha256 ||=
    crypto.createHash("sha256").update(selectionBytes).digest("hex");
}
if (!phase || !selectionPath || !selectionSha256) {
  console.error(
    `${path}: legacy address cannot derive phase and selection digest; ` +
    "record a readable selection_path or start a new lifecycle",
  );
  process.exit(1);
}
process.stdout.write(
  [
    value.round, value.lifecycle_id, value.base, value.previous_tip, value.tip,
    phase, selectionPath, selectionSha256,
  ].join("\t"),
);
NODE
}

previous_round=""
previous_dir=""
display_previous_dir=""
op_previous_dir=""
previous_selection_path=""
if [ "$round_number" -eq 1 ]; then
  if [ "$prev_sha" != "$base_sha" ]; then
    echo "round 1 must use the branch base as <prev-tip>" >&2
    echo "  base      $base_sha" >&2
    echo "  prev-tip  $prev_sha" >&2
    exit 2
  fi
else
  previous_round="$wave-r$((round_number - 1))"
  display_previous_dir="$display_panel_root/$previous_round"
  previous_dir="$display_previous_dir"
  op_previous_dir="$panel_root/$previous_round"
  previous_address="$op_previous_dir/address.json"
  if [ ! -f "$previous_address" ]; then
    echo "missing previous review address: $display_previous_dir/address.json" >&2
    echo "stage reviews sequentially so the incremental range is derived from recorded evidence" >&2
    exit 2
  fi
  if [ -f "$op_previous_dir/.complete" ]; then
    previous_completion_schema="$(
      node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { readFileNoFollow } =
  await import(pathToFileURL(process.argv[2]).href);
const value = JSON.parse(readFileNoFollow(process.argv[1], {
  encoding: "utf8",
  label: "previous completion marker",
}));
process.stdout.write(String(value.schema_version ?? ""));
' "$op_previous_dir/.complete" "$lifecycle_helper"
    )" || {
      echo "previous review has an unreadable completion marker" >&2
      exit 2
    }
    if [ "$previous_completion_schema" = "2" ] &&
       ! validate_bound_completion "$op_previous_dir/.complete"; then
      echo "previous review failed canonical completion validation" >&2
      exit 2
    fi
  fi
  if ! previous_fields="$(read_address "$previous_address")"; then
    exit 2
  fi
  IFS=$'\t' read -r recorded_round recorded_lifecycle _ _ recorded_tip _recorded_phase recorded_selection recorded_selection_sha <<<"$previous_fields"
  if [ "$recorded_round" != "$previous_round" ]; then
    echo "$display_previous_dir/address.json records round $recorded_round, expected $previous_round" >&2
    exit 2
  fi
  if [ "$recorded_lifecycle" != "$lifecycle" ]; then
    echo "$display_previous_dir/address.json records lifecycle $recorded_lifecycle, expected $lifecycle" >&2
    exit 2
  fi
  if [ "$prev_sha" != "$recorded_tip" ]; then
    echo "incremental range does not start at the previous recorded tip" >&2
    echo "  previous round  $previous_round" >&2
    echo "  recorded tip    $recorded_tip" >&2
    echo "  supplied tip    $prev_sha" >&2
    exit 2
  fi
  if [ -z "$recorded_selection" ] ||
     [ "$recorded_selection" != "$display_previous_dir/selection.json" ] ||
     [ ! -f "$op_previous_dir/selection.json" ]; then
    echo "previous review does not record a readable lifecycle selection" >&2
    exit 2
  fi
  previous_selection_path="$op_previous_dir/selection.json"
  IFS=$'\t' read -r actual_recorded_selection_sha _recorded_selection_bytes \
    <<<"$(secure_digest_size "$previous_selection_path")"
  if [ "$actual_recorded_selection_sha" != "$recorded_selection_sha" ]; then
    echo "previous review selection bytes disagree with address.json" >&2
    exit 2
  fi
  previous_roster="$(
    node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { readSelection } = await import(pathToFileURL(process.argv[2]).href);
process.stdout.write(readSelection(process.argv[1]).roster.join(","));
' "$previous_selection_path" "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"
  )"
fi

if [ ! -f "$selection_path" ]; then
  echo "missing lifecycle selection: $selection_path" >&2
  exit 2
fi

selection_meta="$(
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, fullRange, deltaRange, helperPath, lifecycle] = process.argv.slice(1);
const {
  changedPathsFromGitRange,
  readSelection,
  selectionDigest,
  validateSelectionAgainstTable,
} = await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
if (selection.lifecycle_id !== lifecycle) {
  throw new Error(
    `selection lifecycle ${selection.lifecycle_id} disagrees with staging lifecycle ${lifecycle}`,
  );
}
validateSelectionAgainstTable(selection);
const actualFull = changedPathsFromGitRange(fullRange);
if (selection.phase === "verification") {
  const actualDelta = changedPathsFromGitRange(deltaRange);
  const declaredFull = selection.classification_inputs.full_candidate.changed_paths;
  const declaredDelta = selection.classification_inputs.fix_delta.changed_paths;
  if (actualFull.join("\u0000") !== declaredFull.join("\u0000")) {
    throw new Error(
      `selection full-candidate paths do not match git range ${fullRange}; ` +
      `declared [${declaredFull.join(", ")}], actual [${actualFull.join(", ")}]`,
    );
  }
  if (actualDelta.join("\u0000") !== declaredDelta.join("\u0000")) {
    throw new Error(
      `selection fix-delta paths do not match git range ${deltaRange}; ` +
      `declared [${declaredDelta.join(", ")}], actual [${actualDelta.join(", ")}]`,
    );
  }
} else if (
  actualFull.join("\u0000") !==
  selection.classification_inputs.changed_paths.join("\u0000")
) {
  throw new Error(
    `selection changed paths do not match git range ${fullRange}; ` +
    `declared [${selection.classification_inputs.changed_paths.join(", ")}], ` +
    `actual [${actualFull.join(", ")}]`,
  );
}
process.stdout.write([
  selection.phase,
  selectionDigest(selectionPath),
  selection.roster.join(","),
].join("\t"));
' "$selection_path" "$base_sha..$tip" "$prev_sha..$tip" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" "$lifecycle"
)" || {
  echo "selection validation or git-range derivation failed" >&2
  exit 2
}
IFS=$'\t' read -r phase selection_sha256 selected_roster <<<"$selection_meta"
if [ -z "$candidate_path" ]; then
  echo "--candidate is required so current-candidate.json can be materialized from exact bytes" >&2
  exit 2
fi
if [ "$phase" = "discovery" ] && [ -z "$discovery_request_path" ]; then
  echo "--discovery-request is required before a discovery round can be marked complete" >&2
  exit 2
fi
if [ "$phase" = "verification" ]; then
  if [ -z "$previous_selection_path" ]; then
    echo "verification staging requires a recorded prior selection and verdict directory" >&2
    exit 2
  fi
  if [ -z "$ledger_path" ]; then
    echo "--ledger is required for verification staging so discovery-ledger.json is exact" >&2
    exit 2
  fi
  if [ -z "$responses_path" ]; then
    echo "--responses is required for verification staging so responses.json is exact" >&2
    exit 2
  fi
  if [ -z "$self_verification_path" ]; then
    echo "--self-verification is required for verification staging so self-verification.json is exact" >&2
    exit 2
  fi
  if [ -z "$verification_dir" ]; then
    echo "--verification-dir is required for verification staging so every selected seat has a request before .complete" >&2
    exit 2
  fi
fi
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

if [ ! -f "$evidence_path" ] || [ ! -r "$evidence_path" ] ||
   [ ! -s "$evidence_path" ]; then
  echo "missing, unreadable, or empty finalized validation evidence: $evidence_path" >&2
  exit 2
fi

if [ -n "$reviewer_notes_dir" ]; then
  if ! node --input-type=module - \
    "$reviewer_notes_dir" "$selected_roster" "$lifecycle_helper" <<'NODE'
import { pathToFileURL } from "node:url";
const source = process.argv[2];
const selected = process.argv[3].split(",").filter(Boolean);
const { readDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[4]).href);
let files;
try {
  files = readDirectoryNoFollow(source, {
    label: "finalized reviewer-notes directory",
    nonEmpty: true,
    expectedNames: selected.map((seat) => `${seat}.md`),
  });
} catch (error) {
  console.error(`missing or unreadable finalized reviewer-notes directory: ${source}: ${error.message}`);
  process.exit(1);
}
const expected = selected.map((seat) => `${seat}.md`).sort();
const actual = files.map((entry) => entry.name);
if (
  files.length !== expected.length ||
  actual.some((name, index) => name !== expected[index])
) {
  console.error(
    `finalized reviewer-notes directory must contain exactly one regular Markdown file per selected seat; ` +
    `expected [${expected.join(", ")}], found [${actual.join(", ")}]`,
  );
  process.exit(1);
}
NODE
  then
    exit 2
  fi
fi

readable_json_file() {
  local path="$1"
  local label="$2"
  if [ ! -f "$path" ] || [ ! -r "$path" ]; then
    echo "missing or unreadable supplied $label: $path" >&2
    exit 2
  fi
  if ! node --input-type=module - "$path" "$lifecycle_helper" <<'NODE'
import { pathToFileURL } from "node:url";
const path = process.argv[2];
const { readFileNoFollow } =
  await import(pathToFileURL(process.argv[3]).href);
try {
  JSON.parse(readFileNoFollow(path, {
    encoding: "utf8",
    label: "supplied JSON artifact",
  }));
} catch (error) {
  console.error(`${path}: supplied JSON artifact is not readable: ${error.message}`);
  process.exit(1);
}
NODE
  then
    exit 2
  fi
}

if [ "$phase" = "discovery" ]; then
  readable_json_file "$discovery_request_path" "discovery request"
else
  readable_json_file "$ledger_path" "discovery ledger"
  readable_json_file "$responses_path" "implementation responses"
  readable_json_file "$self_verification_path" "self-verification"
  if ! node --input-type=module - \
    "$verification_dir" "$selected_roster" "$lifecycle_helper" <<'NODE'
import { pathToFileURL } from "node:url";
const source = process.argv[2];
const selected = process.argv[3].split(",").filter(Boolean);
const { readDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[4]).href);
let entries;
try {
  entries = readDirectoryNoFollow(source, {
    label: "verification request directory",
    expectedNames: selected.map((seat) => `${seat}.json`),
  });
} catch (error) {
  console.error(`missing or unreadable verification request directory: ${source}: ${error.message}`);
  process.exit(1);
}
const expected = selected.map((seat) => `${seat}.json`).sort();
const actual = entries.map((entry) => entry.name);
if (
  entries.length !== expected.length ||
  actual.some((name, index) => name !== expected[index])
) {
  console.error(
    `verification request directory must contain exactly one readable JSON request per selected seat; ` +
    `expected [${expected.join(", ")}], found [${actual.join(", ")}]`,
  );
  process.exit(1);
}
for (const entry of entries) {
  try {
    JSON.parse(entry.bytes.toString("utf8"));
  } catch (error) {
    console.error(`${source}/${entry.name}: verification request is not readable JSON: ${error.message}`);
    process.exit(1);
  }
}
NODE
  then
    exit 2
  fi
fi

if [ "$round_number" -gt 1 ]; then
  for seat in "${panel_seats[@]}"; do
    case ",$previous_roster," in
      *,"$seat",*)
        prior_verdict="$op_previous_dir/verdicts/$seat.json"
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

for path_value in "$candidate_path" "$discovery_request_path" "$evidence_path" \
  "$reviewer_notes_dir" "$ledger_path" "$responses_path" \
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
     [ "$existing_selection" != "$display_staged_selection_path" ] ||
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

reuse_existing=false
enforce_panel_root_quota() {
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [panelRoot, maxText, helperPath] = process.argv.slice(1);
const { directoryTreeUsageNoFollow } =
  await import(pathToFileURL(helperPath).href);
const usage = directoryTreeUsageNoFollow(panelRoot, {
  label: "panel packet root",
  maxBytes: Number(maxText),
});
process.stdout.write(String(usage.bytes));
' "$panel_root" "$panel_root_max_bytes" "$lifecycle_helper"
}

claim_round_directory() {
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { ensureDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[2]).href);
ensureDirectoryNoFollow(process.argv[1]);
' "$panel_root" "$lifecycle_helper"
  if [ -e "$out" ]; then
    if [ ! -f "$completion_marker" ]; then
      partial_cleanup_hint
      exit 2
    fi
    reuse_existing=true
    return
  fi
  if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { ensureDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[2]).href);
ensureDirectoryNoFollow(process.argv[1], { exclusive: true });
' "$out" "$lifecycle_helper"
  then
    if [ -f "$completion_marker" ]; then
      reuse_existing=true
      return
    fi
    partial_cleanup_hint
    exit 2
  fi
  round_directory_owned=true
}

if ! enforce_panel_root_quota >/dev/null; then
  echo "panel packet root quota refused staging before round materialization" >&2
  exit 2
fi
claim_round_directory

require_reused_path() {
  local path="$1"
  if [ ! -e "$path" ]; then
    echo "complete review $round is missing canonical artifact $path; refusing to add it after .complete" >&2
    return 1
  fi
}

if [ "$reuse_existing" = true ]; then
  for required_path in \
    "$staged_selection_path" \
    "$staged_candidate_path" \
    "$out/delta.diff" \
    "$out/full.diff" \
    "$out/commits.txt" \
    "$out/address.json" \
    "$out/evidence.md" \
    "$out/review-request.md" \
    "$out/dispatch-prompt.txt" \
    "$staged_agent_definitions_dir" \
    "$out/verdicts"; do
    require_reused_path "$required_path" || exit 2
  done
  if [ "$phase" = "discovery" ]; then
    require_reused_path "$staged_discovery_request_path" || exit 2
  else
    for required_path in \
      "$staged_ledger_path" \
      "$staged_responses_path" \
      "$staged_self_verification_path" \
      "$staged_verification_dir"; do
      require_reused_path "$required_path" || exit 2
    done
  fi
  for seat in "${panel_seats[@]}"; do
    require_reused_path \
      "$staged_agent_definitions_dir/panel-$seat.agent.md" || exit 2
    require_reused_path "$out/reviewer-notes/$seat.md" || exit 2
  done
else
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { ensureDirectoryNoFollow } =
  await import(pathToFileURL(process.argv[4]).href);
ensureDirectoryNoFollow(process.argv[1]);
ensureDirectoryNoFollow(process.argv[2]);
ensureDirectoryNoFollow(process.argv[3]);
' "$out/verdicts" "$out/reviewer-notes" "$staged_agent_definitions_dir" \
    "$lifecycle_helper"
fi

publish_stdin_no_replace() {
  local dest="$1"
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [destination, helperPath, retentionRoot, maxBytesText] =
  process.argv.slice(1);
const { writeStandardInputCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
writeStandardInputCreateOrCompare(destination, {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$dest" "$lifecycle_helper" "$panel_root" "$panel_root_max_bytes"
}

materialize_exact() {
  local source="$1"
  local destination="$2"
  local label="$3"
  if [ ! -f "$source" ] || [ ! -r "$source" ]; then
    echo "missing or unreadable supplied $label: $source" >&2
    return 1
  fi
  if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [source, destination, helperPath, retentionRoot, maxBytesText] =
  process.argv.slice(1);
const { copyFileCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
copyFileCreateOrCompare(source, destination, {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$source" "$destination" "$lifecycle_helper" "$panel_root" \
    "$panel_root_max_bytes"
  then
    echo "could not materialize supplied $label at $destination" >&2
    return 1
  fi
}

stage() {
  local dest="$1"
  shift
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [destination, helperPath, retentionRoot, maxBytesText, command, ...args] =
  process.argv.slice(1);
const { writeCommandOutputCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
writeCommandOutputCreateOrCompare(destination, command, args, {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$dest" "$lifecycle_helper" "$panel_root" "$panel_root_max_bytes" "$@"
}

stage "$out/delta.diff" git --no-pager diff "$prev_sha..$tip"
stage "$out/full.diff" git --no-pager diff "$base_sha..$tip"
stage "$out/commits.txt" git --no-pager log --no-decorate --oneline "$base_sha..$tip"

IFS=$'\t' read -r delta_sha _delta_bytes \
  <<<"$(secure_digest_size "$out/delta.diff")"
IFS=$'\t' read -r full_sha _full_bytes \
  <<<"$(secure_digest_size "$out/full.diff")"

materialize_exact "$selection_path" "$staged_selection_path" "lifecycle selection"
IFS=$'\t' read -r staged_selection_sha256 _staged_selection_bytes \
  <<<"$(secure_digest_size "$staged_selection_path")"
if [ "$staged_selection_sha256" != "$selection_sha256" ]; then
  echo "lifecycle selection changed between validation and exact materialization" >&2
  exit 2
fi
if [ -n "$candidate_path" ]; then
  materialize_exact "$candidate_path" "$staged_candidate_path" "current candidate"
else
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, candidatePath, helperPath, retentionRoot, maxBytesText] =
  process.argv.slice(1);
const { candidateFromSelection, readSelection, writeCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
writeCreateOrCompare(candidatePath, candidateFromSelection(selection), {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$staged_selection_path" "$staged_candidate_path" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
  "$panel_root" "$panel_root_max_bytes"
fi
node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, candidatePath, helperPath] = process.argv.slice(1);
const { readFileNoFollow, readSelection, validateSelectionCandidate } =
  await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
validateSelectionCandidate(
  selection,
  JSON.parse(readFileNoFollow(candidatePath, {
    encoding: "utf8",
    label: "staged current candidate",
  })),
);
' "$staged_selection_path" "$staged_candidate_path" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"

materialize_exact "$evidence_path" "$out/evidence.md" \
  "finalized validation evidence"
IFS=$'\t' read -r evidence_sha evidence_bytes \
  <<<"$(secure_digest_size "$out/evidence.md")"

if [ -n "$discovery_request_path" ]; then
  if ! node --input-type=module - \
    "$discovery_request_path" "$staged_discovery_request_path" "$evidence_sha" \
    "$evidence_bytes" \
    "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
    "$panel_root" "$panel_root_max_bytes" <<'NODE'
import { pathToFileURL } from "node:url";
const [source, destination, evidenceSha256, evidenceBytesText, helperPath] =
  process.argv.slice(2, 7);
const [retentionRoot, maxBytesText] = process.argv.slice(7);
const { readFileNoFollow, writeCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
const request = JSON.parse(readFileNoFollow(source, {
  encoding: "utf8",
  label: "generated discovery request",
}));
if (!Array.isArray(request.validation_evidence)) {
  throw new Error("generated discovery request validation_evidence must be an array");
}
const descriptor = {
  artifact_kind: "d2b-panel/validation-evidence",
  path: "evidence.md",
  sha256: evidenceSha256,
  size_bytes: Number(evidenceBytesText),
};
const priorDescriptors = request.validation_evidence.filter((entry) =>
  entry?.artifact_kind === descriptor.artifact_kind &&
  entry?.path === descriptor.path
);
const descriptorMatches = (entry) =>
  entry &&
  typeof entry === "object" &&
  !Array.isArray(entry) &&
  Object.keys(entry).sort().join("\0") ===
    Object.keys(descriptor).sort().join("\0") &&
  Object.entries(descriptor).every(([key, value]) => entry[key] === value);
if (
  priorDescriptors.length > 1 ||
  (priorDescriptors.length === 1 && !descriptorMatches(priorDescriptors[0]))
) {
  throw new Error(
    "generated discovery request carries conflicting validation-evidence bytes",
  );
}
if (priorDescriptors.length === 0) {
  request.validation_evidence.push(descriptor);
}
writeCreateOrCompare(destination, request, {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
NODE
  then
    echo "cannot generate evidence-bound discovery request from $discovery_request_path" >&2
    exit 2
  fi
fi
if [ -n "$ledger_path" ]; then
  materialize_exact "$ledger_path" "$staged_ledger_path" "discovery ledger"
fi
if [ -n "$responses_path" ]; then
  materialize_exact "$responses_path" "$staged_responses_path" \
    "implementation responses"
fi
if [ -n "$self_verification_path" ]; then
  materialize_exact "$self_verification_path" "$staged_self_verification_path" \
    "self-verification"
fi
if [ -n "$approval_path" ]; then
  materialize_exact "$approval_path" "$staged_approval_path" "approval"
fi
if [ -n "$verification_dir" ]; then
  if [ "$reuse_existing" = true ] && [ ! -d "$staged_verification_dir" ]; then
    echo "complete review $round is missing canonical verification requests at $staged_verification_dir; refusing to add them after .complete" >&2
    exit 1
  fi

  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [
  source,
  destination,
  helperPath,
  selectedRoster,
  retentionRoot,
  maxBytesText,
] = process.argv.slice(1);
const { readDirectoryNoFollow, writeDirectoryCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
const entries = readDirectoryNoFollow(source, {
  label: "verification request directory",
  expectedNames: selectedRoster.split(",").map((seat) => `${seat}.json`),
});
if (entries.length === 0 || entries.some((entry) =>
  !entry.name.endsWith(".json")
)) {
  throw new Error("verification directory must contain only regular JSON files");
}
writeDirectoryCreateOrCompare(destination, entries.map((entry) => ({
  name: entry.name,
  bytes: entry.bytes,
})), {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$verification_dir" "$staged_verification_dir" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
  "$selected_roster" "$panel_root" "$panel_root_max_bytes"
fi

if [ "$phase" = "discovery" ]; then
  if ! node - "$staged_discovery_request_path" "$evidence_sha" \
    "$evidence_bytes" "$lifecycle_helper" <<'NODE'
const { pathToFileURL } = require("node:url");
const [requestPath, evidenceSha256, evidenceBytesText] = process.argv.slice(2);
(async () => {
const helperPath = process.argv[5];
const { readFileNoFollow } = await import(pathToFileURL(helperPath).href);
const request = JSON.parse(readFileNoFollow(requestPath, {
  encoding: "utf8",
  label: "staged discovery request",
}));
const evidenceBytes = Number(evidenceBytesText);
const expected = {
  artifact_kind: "d2b-panel/validation-evidence",
  path: "evidence.md",
  sha256: evidenceSha256,
  size_bytes: evidenceBytes,
};
const matches = request.validation_evidence?.filter((entry) =>
  entry &&
  typeof entry === "object" &&
  !Array.isArray(entry) &&
  Object.keys(entry).sort().join("\0") ===
    Object.keys(expected).sort().join("\0") &&
  Object.entries(expected).every(([key, value]) => entry[key] === value)
) ?? [];
if (matches.length !== 1) {
  console.error(
    `${requestPath}: discovery request must contain exactly one canonical ` +
    "validation-evidence descriptor for the finalized evidence.md bytes",
  );
  process.exit(1);
}
})().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
NODE
  then
    echo "generated discovery request does not bind finalized validation evidence" >&2
    exit 2
  fi
fi

if ! node --input-type=module - \
  "$phase" "$lifecycle" "$staged_selection_path" "$staged_candidate_path" \
  "$staged_discovery_request_path" "$staged_ledger_path" \
  "$staged_responses_path" "$staged_self_verification_path" \
  "$staged_verification_dir" \
  "$previous_selection_path" "$op_previous_dir" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" <<'NODE'
import { pathToFileURL } from "node:url";

const [
  phase,
  lifecycle,
  selectionPath,
  candidatePath,
  discoveryRequestPath,
  ledgerPath,
  responsesPath,
  selfVerificationPath,
  verificationDir,
  previousSelectionPath,
  previousDir,
  helperPath,
] = process.argv.slice(2);
const { readDirectoryNoFollow, readFileNoFollow, validateStagedRoundArtifacts } =
  await import(pathToFileURL(helperPath).href);
const readJson = (path, label = "staged panel artifact") =>
  JSON.parse(readFileNoFollow(path, { encoding: "utf8", label }));
const selection = readJson(selectionPath);
const artifacts = {
  phase,
  lifecycle_id: lifecycle,
  selection,
  current_candidate: readJson(candidatePath),
};
if (phase === "discovery") {
  artifacts.discovery_request = readJson(discoveryRequestPath);
} else {
  artifacts.ledger = readJson(ledgerPath);
  artifacts.responses = readJson(responsesPath);
  artifacts.self_verification = readJson(selfVerificationPath);
  const entries = readDirectoryNoFollow(verificationDir, {
    label: "staged verification request directory",
    expectedNames: selection.roster.map((seat) => `${seat}.json`),
  });
  artifacts.verification_requests = Object.fromEntries(
    entries.map((entry) => [
      entry.name.slice(0, -5),
      JSON.parse(entry.bytes.toString("utf8")),
    ]),
  );
}
const validationOptions = {};
if (phase === "verification" && previousSelectionPath) {
  const priorSelection = readJson(previousSelectionPath);
  validationOptions.prior_selection = priorSelection;
  validationOptions.previous_statuses = Object.fromEntries(
    priorSelection.roster.map((seat) => [
      seat,
      readJson(`${previousDir}/verdicts/${seat}.json`),
    ]),
  );
}
validateStagedRoundArtifacts(artifacts, validationOptions);
NODE
then
  echo "staged panel artifacts failed strict lifecycle validation; .complete will not be written" >&2
  exit 2
fi

node --input-type=module -e '
import { pathToFileURL } from "node:url";
const args = process.argv.slice(1);
const maxBytesText = args.pop();
const retentionRoot = args.pop();
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
}, {
  retentionRoot,
  maxBytes: Number(maxBytesText),
});
' "$out/address.json" "$round" "$lifecycle" "$display_staged_selection_path" "$base_sha" \
  "$prev_sha" "$tip" "$phase" "$selection_sha256" "$delta_sha" "$full_sha" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
  "$panel_root" "$panel_root_max_bytes"

for seat in "${panel_seats[@]}"; do
  if [ "$reuse_existing" != true ]; then
    materialize_exact \
      "$root/.github/agents/panel-$seat.agent.md" \
      "$staged_agent_definitions_dir/panel-$seat.agent.md" \
      "panel agent definition for $seat"
  fi

  note="$out/reviewer-notes/$seat.md"
  if [ -n "$reviewer_notes_dir" ]; then
    materialize_exact "$reviewer_notes_dir/$seat.md" "$note" \
      "finalized reviewer note for $seat"
    continue
  fi
  if [ "$reuse_existing" = true ]; then
    # Canonical completion validation above already proved the generated
    # default note's exact bytes. Reuse never recreates or edits it.
    continue
  fi
  if [ "$round_number" -gt 1 ]; then
    case ",$previous_roster," in
      *,"$seat",*)
        prior_note="Read your previous verdict at \`$previous_dir/verdicts/$seat.json\` and verify every prior recommendation against the current tree. Any content change invalidated the prior sign-off."
        ;;
      *)
        prior_note="This is a newly selected seat. No prior verdict exists for this seat; do not invent one. Your first verification obligation is to inspect the complete ledger, every response and its evidence, the full candidate, the latest delta, and this seat's focus before returning a complete status result."
        ;;
    esac
  else
    prior_note="This is the first review. There is no prior verdict; perform the complete discovery obligation for this phase."
  fi
  publish_stdin_no_replace "$note" <<MD
# Reviewer notes for $seat

## Integrator rebuttals

None.

If a prior finding is disputed, replace "None." with the rebuttal and its
evidence. The reviewer may withdraw an incorrect finding and is not required
to withdraw a correct one.

## Reviewer-specific validation request

None. Reviewers do not rerun tests, builds, evals, exploits, or other long
validations unless this section explicitly asks this seat to do so.

## Prior-verdict obligation for $seat

$prior_note
MD
done

discovery_request_path="$staged_discovery_request_path"
ledger_path="$staged_ledger_path"
responses_path="$staged_responses_path"
self_verification_path="$staged_self_verification_path"
verification_dir="$staged_verification_dir"
approval_path="$staged_approval_path"
display_discovery_request_path="$display_staged_discovery_request_path"
display_ledger_path="$display_staged_ledger_path"
display_responses_path="$display_staged_responses_path"
display_self_verification_path="$display_staged_self_verification_path"
display_verification_dir="$display_staged_verification_dir"
display_approval_path="$display_staged_approval_path"

emit_verdict_contract() {
  if [ "$phase" = "discovery" ]; then
    cat <<'MD'
The discovery verdict has exactly four top-level fields. It does not contain
`verified_issue_statuses` or `late_findings`. Replace `<your-seat>` with the
selected seat named by the bound panel agent definition:

```json
{
  "engineer": "<your-seat>",
  "signoff": true,
  "summary": "What was reviewed and the overall posture.",
  "recommendations": []
}
```

Each non-empty `recommendations` entry has exactly `severity`, `where`, `what`,
`why`, and `fix`. Severity is exactly `critical`, `high`, `medium`, or `low`.
`signoff` is true if and only if `recommendations` is empty.
MD
    return
  fi

  cat <<'MD'
The verification verdict has exactly six top-level fields:
`engineer`, `signoff`, `summary`, `verified_issue_statuses`, `late_findings`,
and `recommendations`. Use this exact schema:

MD
  node --input-type=module - "$ledger_path" "$lifecycle_helper" <<'NODE'
import { pathToFileURL } from "node:url";
const [ledgerPath, helperPath] = process.argv.slice(2);
const { readFileNoFollow } =
  await import(pathToFileURL(helperPath).href);
const ledger = JSON.parse(readFileNoFollow(ledgerPath, {
  encoding: "utf8",
  label: "staged discovery ledger",
}));
const statuses = Object.fromEntries(
  ledger.issues.map((issue) => [issue.id, "verified"]),
);
const example = {
  engineer: "<your-seat>",
  signoff: true,
  summary: "What was verified and the overall posture.",
  verified_issue_statuses: statuses,
  late_findings: [],
  recommendations: [],
};
process.stdout.write(`\`\`\`json\n${JSON.stringify(example, null, 2)}\n\`\`\`\n`);
NODE
  cat <<'MD'

Replace `<your-seat>` with the selected seat named by the bound panel agent
definition.

`verified_issue_statuses` has exactly one entry for every ledger issue. Use
`verified` or `resolved` only when confirmed; every other status remains
blocking. `late_findings` is always present and is an array. Each non-empty
`recommendations` entry has exactly `severity`, `where`, `what`, `why`, and
`fix`. `signoff` is true if and only if `recommendations` is empty.
MD
}
verdict_contract="$(emit_verdict_contract)"

{
cat <<MD
# Panel review request

This is the complete shared request for \`$round\` in lifecycle \`$lifecycle\`. Read the artifacts below
with \`view\`; do not substitute a prose summary for them.

## Review address

- Stage completion marker: \`$display_completion_marker\` (this request is usable only
  when that marker exists)
- Delta to review: \`$display_out/delta.diff\`
- Delta range: \`$prev_sha..$tip\`
- Full branch context: \`$display_out/full.diff\`
- Full range: \`$base_sha..$tip\`
- Phase: \`$phase\`
- Lifecycle selection: \`$display_staged_selection_path\` (sha256 \`$selection_sha256\`)
- Staged current candidate: \`$display_staged_candidate_path\`
- Bound panel agent definition:
  \`$display_staged_agent_definitions_dir/panel-<your-seat>.agent.md\`
- Validation evidence and phase deliverable: \`$display_out/evidence.md\`
  (sha256 \`$evidence_sha\`, bound by the completion marker)
- Seat-specific notes: \`$display_out/reviewer-notes/<your-seat>.md\`
- Commit list: \`$display_out/commits.txt\`

## Generated lifecycle artifacts

The canonical generated artifacts for this phase are:

$(if [ "$phase" = "discovery" ]; then
  printf '%s\n' \
  "- Discovery request: \`$display_discovery_request_path\`"
else
  printf '%s\n' \
  "- Immutable discovery ledger: \`$display_ledger_path\`" \
  "- Implementation responses: \`$display_responses_path\`" \
  "- Self-verification: \`$display_self_verification_path\`" \
  "- Verification requests: \`$display_verification_dir/<your-seat>.json\`" \
  "- Approval output after verdict collection: \`$display_approval_path\`"
fi)

## Required review behaviour

1. Read the full candidate in \`$display_out/full.diff\` in full. On discovery, this
   full candidate is the review target, not only the incremental delta. Report
   every reasonably discoverable actionable finding now; do not save
   observations for later rounds.
2. Read the incremental delta in \`$display_out/delta.diff\` as well. On verification,
   review it for resolution, regressions, and unsafe late BLOCKER or MAJOR
   findings without reopening comprehensive discovery.
3. Read the validation evidence and phase deliverable. Missing or insufficient
   coverage is a finding. Do not rerun validation unless your seat-specific
   notes explicitly ask you to.
4. Read your seat-specific notes. Judge any rebuttal on its merits.
5. Read your bound panel agent definition for the seat focus, finding bar, and
   read-only review rules. The active phase and verdict schema in this request
   are authoritative over any inactive-phase example in that definition.
6. Inspect the tree and the diff rather than trusting a summary of what was
   intended to change.
7. Confine findings to defects in the candidate or delta that would cause incorrect
   behaviour, mask a regression, or weaken a stated invariant. Put other
   observations in the summary.
8. Return exactly the phase-specific JSON verdict below and no other text.

## Required $phase verdict contract

$verdict_contract
MD

if [ "$round_number" -gt 1 ]; then
  cat <<MD

## Prior verdict obligation

- For an incumbent seat, your previous verdict is
  \`$previous_dir/verdicts/<your-seat>.json\`. Read it and verify every prior
  recommendation against the current tree.
- For a newly selected seat, no prior verdict exists. Do not invent one; your
  first verification must inspect the complete ledger, every response and its
  evidence, the full candidate, the latest delta, and the seat-specific focus.
- Previous reviewed tip: \`$prev_sha\`

Do not mark a finding resolved because the integrator says it was fixed. Any content change invalidated every prior sign-off, including a sign-off from a seat whose area appears unaffected.
MD
else
  cat <<'MD'

## Prior verdict obligation

This is the first review. There is no prior verdict to verify.
MD
fi
} | publish_stdin_no_replace "$out/review-request.md"

{
cat <<MD
Use this dispatch prompt only when the stage completion marker exists at $display_completion_marker. If it is absent, the scratch directory is non-authoritative and must be cleaned up before retrying.

This is the $phase phase. Read and follow the complete immutable review request at $display_out/review-request.md. Use view to read every artifact it names, including the bound panel agent definition at $display_staged_agent_definitions_dir/panel-<your-seat>.agent.md, the staged current candidate, generated lifecycle artifacts, the delta, and your seat-specific notes. The active phase and verdict contract below are authoritative over any inactive-phase example in the agent definition. Review the delta rather than a prose summary, and return only the exact JSON object required below.

Required $phase verdict contract:

$verdict_contract
MD
} | publish_stdin_no_replace "$out/dispatch-prompt.txt"

canonical_artifacts=(
  "address.json"
  "commits.txt"
  "current-candidate.json"
  "delta.diff"
  "dispatch-prompt.txt"
  "evidence.md"
  "full.diff"
  "review-request.md"
  "selection.json"
)
if [ "$phase" = "discovery" ]; then
  canonical_artifacts+=("discovery-request.json")
else
  canonical_artifacts+=(
    "discovery-ledger.json"
    "responses.json"
    "self-verification.json"
  )
fi
for seat in "${panel_seats[@]}"; do
  canonical_artifacts+=("agent-definitions/panel-$seat.agent.md")
  canonical_artifacts+=("reviewer-notes/$seat.md")
  if [ "$phase" = "verification" ]; then
    canonical_artifacts+=("verification/$seat.json")
  fi
done

if ! enforce_panel_root_quota >/dev/null; then
  echo "panel packet root quota refused completion; .complete will not be written" >&2
  exit 2
fi
if ! verify_locked_panel_root; then
  echo "panel packet root pathname no longer names the locked identity; .complete will not be written" >&2
  exit 2
fi

node --input-type=module - "$lifecycle_helper" "$out" \
  "${canonical_artifacts[@]}" <<'NODE'
import { pathToFileURL } from "node:url";
const [helperPath, root, ...relativePaths] = process.argv.slice(2);
const { chmodFileNoFollow } = await import(pathToFileURL(helperPath).href);
for (const relative of relativePaths) {
  chmodFileNoFollow(`${root}/${relative}`, 0o444);
}
NODE

if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [path, round, lifecycle, base, previousTip, tip, phase, selectionSha,
  deltaSha, fullSha, helperPath, ...artifactPaths] = process.argv.slice(1);
const maxBytes = Number(artifactPaths.pop());
const retentionRoot = artifactPaths.pop();
const { writeBoundCompletionCreateOrCompare } =
  await import(pathToFileURL(helperPath).href);
writeBoundCompletionCreateOrCompare(path, {
  artifact_kind: "d2b-panel/stage-completion",
  schema_version: 2,
  complete: true,
  round,
  lifecycle_id: lifecycle,
  base,
  previous_tip: previousTip,
  tip,
  phase,
  selection_sha256: selectionSha,
  delta_sha256: deltaSha,
  full_sha256: fullSha,
}, artifactPaths, {
  retentionRoot,
  maxBytes,
});
' "$completion_marker" "$round" "$lifecycle" "$base_sha" "$prev_sha" "$tip" \
  "$phase" "$selection_sha256" "$delta_sha" "$full_sha" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
  "${canonical_artifacts[@]}" "$panel_root" "$panel_root_max_bytes"
then
  echo "panel packet root quota or atomic completion publication refused .complete" >&2
  exit 2
fi
node --input-type=module -e '
import { pathToFileURL } from "node:url";
const { chmodFileNoFollow } =
  await import(pathToFileURL(process.argv[2]).href);
chmodFileNoFollow(process.argv[1], 0o444);
' "$completion_marker" "$lifecycle_helper"

echo "staged $display_out"
echo "  tip          $tip"
echo "  delta        $prev_sha..$tip  ($delta_sha)"
echo "  full         $base_sha..$tip  ($full_sha)"
echo
echo "Finalized evidence and reviewer notes are byte-bound by $display_completion_marker."
echo "Dispatch every seat with the exact contents of $display_out/dispatch-prompt.txt."
