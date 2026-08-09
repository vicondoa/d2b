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
dispatch_policy_path="$root/.github/skills/d2b-panel-round/dispatch-policy.json"

tip="$(git rev-parse HEAD)"
base_sha="$(git rev-parse "$base")"
prev_sha="$(git rev-parse "$prev")"

display_panel_root="$root/.scratch/panel"
panel_root="$display_panel_root"
out="$panel_root/$round"
completion_marker="$out/.complete"
staged_selection_path="$out/selection.json"
staged_candidate_path="$out/current-candidate.json"
staged_dispatch_binding_path="$out/dispatch-binding.json"
staged_discovery_request_path="$out/discovery-request.json"
staged_ledger_path="$out/discovery-ledger.json"
staged_responses_path="$out/responses.json"
staged_self_verification_path="$out/self-verification.json"
staged_approval_path="$out/approval.json"
staged_verification_dir="$out/verification"
staged_agent_definitions_dir="$out/agent-definitions"
display_out="$out"
display_completion_marker="$completion_marker"
display_staged_selection_path="$staged_selection_path"
display_staged_candidate_path="$staged_candidate_path"
display_staged_dispatch_binding_path="$staged_dispatch_binding_path"
display_staged_discovery_request_path="$staged_discovery_request_path"
display_staged_ledger_path="$staged_ledger_path"
display_staged_responses_path="$staged_responses_path"
display_staged_self_verification_path="$staged_self_verification_path"
display_staged_approval_path="$staged_approval_path"
display_staged_verification_dir="$staged_verification_dir"
display_staged_agent_definitions_dir="$staged_agent_definitions_dir"
mkdir -p "$display_panel_root"
panel_root="$display_panel_root"
out="$panel_root/$round"
completion_marker="$out/.complete"
staged_selection_path="$out/selection.json"
staged_candidate_path="$out/current-candidate.json"
staged_dispatch_binding_path="$out/dispatch-binding.json"
staged_discovery_request_path="$out/discovery-request.json"
staged_ledger_path="$out/discovery-ledger.json"
staged_responses_path="$out/responses.json"
staged_self_verification_path="$out/self-verification.json"
staged_approval_path="$out/approval.json"
staged_verification_dir="$out/verification"
staged_agent_definitions_dir="$out/agent-definitions"

canonical_artifact_names() {
  local artifact_phase="$1"
  local artifact_roster="$2"
  printf '%s\n' \
    "address.json" \
    "commits.txt" \
    "current-candidate.json" \
    "delta.diff" \
    "dispatch-binding.json" \
    "dispatch-prompt.txt" \
    "evidence.md" \
    "full.diff" \
    "review-request.md" \
    "selection.json"
  if [ "$artifact_phase" = "discovery" ]; then
    printf '%s\n' "discovery-request.json"
  elif [ "$artifact_phase" = "verification" ]; then
    printf '%s\n' \
      "discovery-ledger.json" \
      "responses.json" \
      "self-verification.json"
  else
    echo "unknown panel phase for canonical artifact names: $artifact_phase" >&2
    return 2
  fi
  local -a artifact_seats=()
  local seat
  IFS=',' read -r -a artifact_seats <<<"$artifact_roster"
  for seat in "${artifact_seats[@]}"; do
    [ -n "$seat" ] || {
      echo "empty panel seat in canonical artifact roster" >&2
      return 2
    }
    printf '%s\n' "agent-definitions/panel-$seat.agent.md"
    printf '%s\n' "reviewer-notes/$seat.md"
    if [ "$artifact_phase" = "verification" ]; then
      printf '%s\n' "verification/$seat.json"
    fi
  done
}

validate_bound_completion() {
  local marker="$1"
  local expected_phase="$2"
  shift 2
  node --input-type=module - "$marker" "$expected_phase" "$@" <<'NODE'
import crypto from "node:crypto";
import path from "node:path";
import { readFileSync, statSync } from "node:fs";
const markerPath = process.argv[2];
const expectedPhase = process.argv[3];
const expectedNames = process.argv.slice(4).sort();
const MAX_COMPLETION_MARKER_BYTES = 256 * 1024;
const MAX_BOUND_ARTIFACT_BYTES = 64 * 1024 * 1024;
const MAX_AGENT_DEFINITION_BYTES = 1024 * 1024;
const readBounded = (file, label, maximum) => {
  const stat = statSync(file);
  if (!stat.isFile()) {
    throw new Error(`${label} is not a regular file`);
  }
  if (stat.size > maximum) {
    throw new Error(`${label} exceeds ${maximum} bytes`);
  }
  const bytes = readFileSync(file);
  if (bytes.length > maximum) {
    throw new Error(`${label} exceeds ${maximum} bytes`);
  }
  return bytes;
};
let marker;
try {
  marker = JSON.parse(
    readBounded(
      markerPath,
      "completion marker",
      MAX_COMPLETION_MARKER_BYTES,
    ).toString("utf8"),
  );
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
  ![2, 3].includes(marker.schema_version) ||
  marker.complete !== true ||
  !marker.artifact_sha256 ||
  !marker.artifact_bytes ||
  Object.keys(marker).sort().join("\0") !== exactKeys.join("\0")
) {
  console.error(
    `${markerPath}: completion marker is not a supported canonical byte-bound marker`,
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
const actualNames = Object.keys(digests).sort();
const compatibleExpectedNames = marker.schema_version === 2
  ? expectedNames.filter(
      (name) =>
        !name.startsWith("agent-definitions/") &&
        name !== "dispatch-binding.json",
    )
  : expectedNames;
if (
  marker.phase !== expectedPhase ||
  ![2, 3].includes(marker.schema_version) ||
  compatibleExpectedNames.length !== actualNames.length ||
  compatibleExpectedNames.some((name, index) => name !== actualNames[index])
) {
  const missing = compatibleExpectedNames.filter((name) => !actualNames.includes(name));
  const extra = actualNames.filter((name) => !compatibleExpectedNames.includes(name));
  console.error(
    `${markerPath}: completion artifact set disagrees with phase and selected roster; ` +
    `missing [${missing.join(", ")}], extra [${extra.join(", ")}]`,
  );
  process.exit(1);
}
const root = path.dirname(markerPath);
for (const relative of actualNames) {
  if (
    relative === "" ||
    path.isAbsolute(relative) ||
    relative.split("/").includes("..") ||
    !/^[0-9a-f]{64}$/.test(digests[relative]) ||
    !Number.isSafeInteger(sizes[relative]) ||
    sizes[relative] < 0 ||
    sizes[relative] >
      (relative.startsWith("agent-definitions/")
        ? MAX_AGENT_DEFINITION_BYTES
        : MAX_BOUND_ARTIFACT_BYTES)
  ) {
    console.error(`${markerPath}: invalid bound artifact entry ${relative}`);
    process.exit(1);
  }
  let bytes;
  try {
    bytes = readBounded(
      path.join(root, relative),
      `bound artifact ${relative}`,
      relative.startsWith("agent-definitions/")
        ? MAX_AGENT_DEFINITION_BYTES
        : MAX_BOUND_ARTIFACT_BYTES,
    );
  } catch (error) {
    console.error(`${markerPath}: bound artifact ${relative} is unavailable: ${error.message}`);
    process.exit(1);
  }
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

validate_bound_selection_entry() {
  local marker="$1"
  local selection="$2"
  node --input-type=module - "$marker" "$selection" <<'NODE'
import crypto from "node:crypto";
import { readFileSync, statSync } from "node:fs";

const markerPath = process.argv[2];
const selectionPath = process.argv[3];
const MAX_COMPLETION_MARKER_BYTES = 256 * 1024;
const MAX_SELECTION_BYTES = 64 * 1024 * 1024;
const readBounded = (path, label, maximum) => {
  const stat = statSync(path);
  if (!stat.isFile()) {
    throw new Error(`${label} is not a regular file`);
  }
  if (stat.size > maximum) {
    throw new Error(`${label} exceeds ${maximum} bytes`);
  }
  const bytes = readFileSync(path);
  if (bytes.length > maximum) {
    throw new Error(`${label} exceeds ${maximum} bytes`);
  }
  return bytes;
};

let marker;
try {
  marker = JSON.parse(
    readBounded(
      markerPath,
      "completion marker",
      MAX_COMPLETION_MARKER_BYTES,
    ).toString("utf8"),
  );
} catch (error) {
  console.error(`${markerPath}: invalid completion marker: ${error.message}`);
  process.exit(1);
}
if (
  !marker ||
  typeof marker !== "object" ||
  Array.isArray(marker) ||
  marker.artifact_kind !== "d2b-panel/stage-completion" ||
  ![2, 3].includes(marker.schema_version) ||
  marker.complete !== true
) {
  console.error(
    `${markerPath}: completion marker is not a supported canonical byte-bound marker`,
  );
  process.exit(1);
}
const digest = marker.artifact_sha256?.["selection.json"];
const size = marker.artifact_bytes?.["selection.json"];
if (
  typeof digest !== "string" ||
  !/^[0-9a-f]{64}$/u.test(digest) ||
  !Number.isSafeInteger(size) ||
  size < 0 ||
  size > MAX_SELECTION_BYTES
) {
  console.error(
    `${markerPath}: completion marker does not carry a valid selection.json binding`,
  );
  process.exit(1);
}
let bytes;
try {
  bytes = readBounded(selectionPath, "bound selection.json", MAX_SELECTION_BYTES);
} catch (error) {
  console.error(
    `${markerPath}: bound selection.json is unavailable: ${error.message}`,
  );
  process.exit(1);
}
const actual = crypto.createHash("sha256").update(bytes).digest("hex");
if (actual !== digest || bytes.length !== size) {
  console.error(
    `${markerPath}: bound selection.json has a different size or digest`,
  );
  process.exit(1);
}
NODE
}

reject_completed_discovery_packet() {
  local panel_root="$1"
  local current_out="$2"
  local lifecycle_id="$3"
  node --input-type=module - "$panel_root" "$current_out" "$lifecycle_id" <<'NODE'
import path from "node:path";
import { existsSync, lstatSync, readdirSync, readFileSync, statSync } from "node:fs";

const panelRoot = process.argv[2];
const currentOut = path.resolve(process.argv[3]);
const lifecycle = process.argv[4];
const MAX_DIRECTORY_ENTRIES = 4096;
const MAX_COMPLETION_MARKER_BYTES = 256 * 1024;
if (!existsSync(panelRoot)) process.exit(0);
const rootStat = lstatSync(panelRoot);
if (!rootStat.isDirectory()) {
  console.error(`${panelRoot}: panel scratch root is not a directory`);
  process.exit(1);
}
const packets = readdirSync(panelRoot).sort();
if (packets.length > MAX_DIRECTORY_ENTRIES) {
  console.error(
    `${panelRoot}: panel scratch root has more than ${MAX_DIRECTORY_ENTRIES} entries`,
  );
  process.exit(1);
}
for (const name of packets) {
  const packet = path.join(panelRoot, name);
  if (path.resolve(packet) === currentOut) continue;
  let packetStat;
  try {
    packetStat = lstatSync(packet);
  } catch (error) {
    if (error.code === "ENOENT") continue;
    throw error;
  }
  if (!packetStat.isDirectory()) continue;
  const markerPath = path.join(packet, ".complete");
  let markerStat;
  try {
    markerStat = statSync(markerPath);
  } catch (error) {
    if (error.code === "ENOENT") continue;
    throw error;
  }
  if (!markerStat.isFile()) continue;
  if (markerStat.size > MAX_COMPLETION_MARKER_BYTES) {
    console.error(
      `${markerPath}: completion marker exceeds ${MAX_COMPLETION_MARKER_BYTES} bytes`,
    );
    process.exit(1);
  }
  const bytes = readFileSync(markerPath);
  if (bytes.length > MAX_COMPLETION_MARKER_BYTES) {
    console.error(
      `${markerPath}: completion marker exceeds ${MAX_COMPLETION_MARKER_BYTES} bytes`,
    );
    process.exit(1);
  }
  let marker;
  try {
    marker = JSON.parse(bytes.toString("utf8"));
  } catch {
    continue;
  }
  if (
    marker &&
    typeof marker === "object" &&
    !Array.isArray(marker) &&
    marker.complete === true &&
    marker.artifact_kind === "d2b-panel/stage-completion" &&
    [2, 3].includes(marker.schema_version) &&
    marker.lifecycle_id === lifecycle &&
    marker.phase === "discovery"
  ) {
    console.error(
      `lifecycle "${lifecycle}" already has a completed discovery packet at ${packet}; ` +
        "discovery is exactly once by lifecycle identity, independent of round prefix",
    );
    process.exit(1);
  }
}
NODE
}

secure_digest_size() {
  node --input-type=module - "$1" <<'NODE'
import crypto from "node:crypto";
import { readFileSync, statSync } from "node:fs";
const path = process.argv[2];
const maximum = 64 * 1024 * 1024;
const stat = statSync(path);
if (!stat.isFile() || stat.size > maximum) {
  throw new Error(`${path}: digest input is not a bounded regular file`);
}
const bytes = readFileSync(path);
if (bytes.length > maximum) {
  throw new Error(`${path}: digest input is oversized`);
}
process.stdout.write(
  `${crypto.createHash("sha256").update(bytes).digest("hex")}\t${bytes.length}`,
);
NODE
}

read_address() {
  node --input-type=module - "$1" <<'NODE'
import { readFileSync, statSync } from "node:fs";
const path = process.argv[2];
let value;
try {
  const maximum = 1024 * 1024;
  const stat = statSync(path);
  if (!stat.isFile() || stat.size > maximum) {
    throw new Error(`address.json is not a bounded regular file`);
  }
  const bytes = readFileSync(path);
  if (bytes.length > maximum) throw new Error("address.json is oversized");
  value = JSON.parse(bytes.toString("utf8"));
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
const phase = value.phase;
const selectionPath = value.selection_path;
const selectionSha256 = value.selection_sha256;
if (
  typeof phase !== "string" ||
  typeof selectionPath !== "string" ||
  typeof selectionSha256 !== "string" ||
  selectionPath.length === 0 ||
  selectionSha256.length === 0
) {
  console.error(`${path}: address.json must record phase, selection_path, and selection_sha256`);
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
  op_previous_dir="$previous_dir"
  previous_selection_path="$op_previous_dir/selection.json"
fi

if [ ! -f "$selection_path" ]; then
  echo "missing lifecycle selection: $selection_path" >&2
  exit 2
fi

selection_identity="$(
  node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, helperPath, lifecycle] = process.argv.slice(1);
const {
  readSelection,
  selectionDigest,
} = await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
if (selection.lifecycle_id !== lifecycle) {
  throw new Error(
    `selection lifecycle ${selection.lifecycle_id} disagrees with staging lifecycle ${lifecycle}`,
  );
}
process.stdout.write([
  selection.phase,
  selectionDigest(selectionPath),
  selection.roster.join(","),
].join("\t"));
' "$selection_path" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" "$lifecycle"
)" || {
  echo "selection validation failed" >&2
  exit 2
}
IFS=$'\t' read -r phase selection_sha256 selected_roster <<<"$selection_identity"
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

if [ "$round_number" -gt 1 ] && [ "$phase" != "verification" ]; then
  echo "round $round requires a verification selection after the completed discovery packet" >&2
  echo "subsequent staging must not run a second discovery" >&2
  exit 2
fi

if [ "$phase" = "discovery" ]; then
  if ! reject_completed_discovery_packet \
    "$display_panel_root" "$out" "$lifecycle"; then
    echo "discovery staging requires exactly one completed packet for this lifecycle" >&2
    exit 2
  fi
fi

if [ "$round_number" -gt 1 ]; then
  if [ ! -f "$op_previous_dir/.complete" ]; then
    echo "missing canonical predecessor packet: $display_previous_dir/.complete" >&2
    echo "later-round staging requires the predecessor completion marker before reading its artifacts" >&2
    exit 2
  fi
  previous_selection_path="$op_previous_dir/selection.json"
  if ! validate_bound_selection_entry \
    "$op_previous_dir/.complete" "$previous_selection_path"; then
    echo "previous review is not a supported canonical completion packet" >&2
    exit 2
  fi
  previous_selection_meta="$(
    node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, lifecycle, helperPath] = process.argv.slice(1);
const { readSelection } = await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
if (selection.lifecycle_id !== lifecycle) {
  throw new Error(
    `previous selection lifecycle ${selection.lifecycle_id} disagrees with staging lifecycle ${lifecycle}`,
  );
}
process.stdout.write([selection.phase, selection.roster.join(",")].join("\t"));
' "$previous_selection_path" "$lifecycle" \
      "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"
  )" || {
    echo "previous review does not record a readable lifecycle selection" >&2
    exit 2
  }
  IFS=$'\t' read -r previous_phase previous_roster <<<"$previous_selection_meta"
  previous_canonical_artifacts=()
  while IFS= read -r artifact_name; do
    previous_canonical_artifacts+=("$artifact_name")
  done < <(canonical_artifact_names "$previous_phase" "$previous_roster")
  if ! validate_bound_completion \
    "$op_previous_dir/.complete" "$previous_phase" \
    "${previous_canonical_artifacts[@]}"; then
    echo "previous review is not a supported canonical completion packet" >&2
    exit 2
  fi
  previous_address="$op_previous_dir/address.json"
  if [ ! -f "$previous_address" ]; then
    echo "missing previous review address: $display_previous_dir/address.json" >&2
    echo "stage reviews sequentially so the incremental range is derived from recorded evidence" >&2
    exit 2
  fi
  if ! previous_fields="$(read_address "$previous_address")"; then
    exit 2
  fi
  IFS=$'\t' read -r recorded_round recorded_lifecycle _ _ recorded_tip recorded_phase recorded_selection recorded_selection_sha <<<"$previous_fields"
  if [ "$recorded_round" != "$previous_round" ]; then
    echo "$display_previous_dir/address.json records round $recorded_round, expected $previous_round" >&2
    exit 2
  fi
  if [ "$recorded_lifecycle" != "$lifecycle" ]; then
    echo "$display_previous_dir/address.json records lifecycle $recorded_lifecycle, expected $lifecycle" >&2
    exit 2
  fi
  if [ "$recorded_phase" != "$previous_phase" ]; then
    echo "$display_previous_dir/address.json records phase $recorded_phase, expected $previous_phase" >&2
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
     [ "$recorded_selection" != "$display_previous_dir/selection.json" ]; then
    echo "previous review does not record a readable lifecycle selection" >&2
    exit 2
  fi
  IFS=$'\t' read -r actual_recorded_selection_sha _recorded_selection_bytes \
    <<<"$(secure_digest_size "$previous_selection_path")"
  if [ "$actual_recorded_selection_sha" != "$recorded_selection_sha" ]; then
    echo "previous review selection bytes disagree with address.json" >&2
    exit 2
  fi
fi

if ! node --input-type=module -e '
import { pathToFileURL } from "node:url";
const [selectionPath, fullRange, deltaRange, helperPath] = process.argv.slice(1);
const {
  changedPathsFromGitRange,
  readSelection,
} = await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
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
' "$selection_path" "$base_sha..$tip" "$prev_sha..$tip" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"
then
  echo "selection validation or git-range derivation failed" >&2
  exit 2
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

canonical_artifacts=()
while IFS= read -r artifact_name; do
  canonical_artifacts+=("$artifact_name")
done < <(canonical_artifact_names "$phase" "$selected_roster")

reuse_existing=false
existing_completion_schema=""
if [ -e "$out" ]; then
  reuse_existing=true
  if [ ! -f "$completion_marker" ]; then
    echo "round $round already has an incomplete packet; remove that exact directory before retrying" >&2
    exit 2
  fi
  if ! validate_bound_completion \
    "$completion_marker" "$phase" "${canonical_artifacts[@]}"; then
    echo "complete review $round failed canonical completion validation" >&2
    exit 2
  fi
  existing_completion_schema="$(
    node --input-type=module - "$completion_marker" <<'NODE'
import { readFileSync, statSync } from "node:fs";
const path = process.argv[2];
const maximum = 256 * 1024;
const stat = statSync(path);
if (!stat.isFile() || stat.size > maximum) {
  throw new Error("completion marker is not a bounded regular file");
}
const bytes = readFileSync(path);
if (bytes.length > maximum) throw new Error("completion marker is oversized");
const marker = JSON.parse(bytes.toString("utf8"));
process.stdout.write(String(marker.schema_version));
NODE
  )" || {
    echo "complete review $round has an unreadable completion marker" >&2
    exit 2
  }
fi

if [ ! -f "$evidence_path" ] || [ ! -r "$evidence_path" ] ||
   [ ! -s "$evidence_path" ]; then
  echo "missing, unreadable, or empty finalized validation evidence: $evidence_path" >&2
  exit 2
fi

if [ -n "$reviewer_notes_dir" ]; then
  if ! node --input-type=module - \
    "$reviewer_notes_dir" "$selected_roster" <<'NODE'
import { readdirSync, readFileSync, statSync } from "node:fs";
const source = process.argv[2];
const selected = process.argv[3].split(",").filter(Boolean);
let files;
try {
  files = readdirSync(source)
    .sort()
    .map((name) => ({
      name,
      size: statSync(`${source}/${name}`).size,
    }));
} catch (error) {
  console.error(`missing or unreadable finalized reviewer-notes directory: ${source}: ${error.message}`);
  process.exit(1);
}
const expected = selected.map((seat) => `${seat}.md`).sort();
const actual = files.map((entry) => entry.name);
if (
  files.length !== expected.length ||
  actual.some((name, index) => name !== expected[index]) ||
  files.some((entry) => entry.size === 0)
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
  if ! node --input-type=module - "$path" <<'NODE'
import { readFileSync } from "node:fs";
const path = process.argv[2];
try {
  JSON.parse(readFileSync(path, "utf8"));
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
    "$verification_dir" "$selected_roster" <<'NODE'
import { readdirSync, readFileSync, statSync } from "node:fs";
const source = process.argv[2];
const selected = process.argv[3].split(",").filter(Boolean);
let entries;
try {
  entries = readdirSync(source)
    .sort()
    .map((name) => {
      const path = `${source}/${name}`;
      const stat = statSync(path);
      return { name, bytes: readFileSync(path), regular: stat.isFile() };
    });
} catch (error) {
  console.error(`missing or unreadable verification request directory: ${source}: ${error.message}`);
  process.exit(1);
}
const expected = selected.map((seat) => `${seat}.json`).sort();
const actual = entries.map((entry) => entry.name);
if (
  entries.length !== expected.length ||
  actual.some((name, index) => name !== expected[index]) ||
  entries.some((entry) => !entry.regular)
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

if [ "$reuse_existing" != true ]; then
  mkdir -p "$out/verdicts" "$out/reviewer-notes"
fi

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
    "$out/verdicts"; do
    require_reused_path "$required_path" || exit 2
  done
  if [ "$existing_completion_schema" = "3" ]; then
    require_reused_path "$staged_dispatch_binding_path" || exit 2
    require_reused_path "$staged_agent_definitions_dir" || exit 2
  fi
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
    if [ "$existing_completion_schema" = "3" ]; then
      require_reused_path "$staged_agent_definitions_dir/panel-$seat.agent.md" || exit 2
    fi
    require_reused_path "$out/reviewer-notes/$seat.md" || exit 2
  done
fi

publish_stdin_no_replace() {
  local dest="$1"
  node --input-type=module -e '
import { dirname } from "node:path";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
const destination = process.argv[1];
const expected = readFileSync(0);
mkdirSync(dirname(destination), { recursive: true });
let actual;
try {
  actual = readFileSync(destination);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (actual !== undefined) {
  if (!actual.equals(expected)) {
    throw new Error(`conflicting generated bytes at ${destination}`);
  }
  process.exit(0);
}
try {
  writeFileSync(destination, expected, { flag: "wx" });
} catch (error) {
  if (error.code !== "EEXIST") throw error;
  actual = readFileSync(destination);
  if (!actual.equals(expected)) {
    throw new Error(`conflicting generated bytes at ${destination}`);
  }
}
' "$dest"
}

materialize_exact() {
  local source="$1"
  local destination="$2"
  local label="$3"
  if [ ! -f "$source" ] || [ ! -r "$source" ]; then
    echo "missing or unreadable supplied $label: $source" >&2
    return 1
  fi
  if ! publish_stdin_no_replace "$destination" < "$source"; then
    echo "could not materialize supplied $label at $destination" >&2
    return 1
  fi
}

publish_directory() {
  local source="$1"
  local destination="$2"
  local selected="$3"
  node --input-type=module -e '
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync, renameSync } from "node:fs";
import { dirname, join } from "node:path";
const [source, destination, selectedText] = process.argv.slice(1);
const expectedNames = selectedText.split(",").filter(Boolean).map((seat) => `${seat}.json`).sort();
const entries = readdirSync(source).sort();
if (
  entries.length !== expectedNames.length ||
  entries.some((name, index) => name !== expectedNames[index]) ||
  entries.some((name) => !statSync(join(source, name)).isFile())
) {
  throw new Error(
    `verification request directory must contain exactly one regular JSON file per selected seat; ` +
    `expected [${expectedNames.join(", ")}], found [${entries.join(", ")}]`,
  );
}
const expected = new Map(entries.map((name) => [name, readFileSync(join(source, name))]));
const compare = (path) => {
  const actualNames = readdirSync(path).sort();
  if (
    actualNames.length !== expectedNames.length ||
    actualNames.some((name, index) => name !== expectedNames[index])
  ) {
    throw new Error(`existing artifact family at ${destination} is incomplete or has extra entries`);
  }
  for (const name of expectedNames) {
    if (!readFileSync(join(path, name)).equals(expected.get(name))) {
      throw new Error(`conflicting generated bytes at ${join(destination, name)}`);
    }
  }
};
mkdirSync(dirname(destination), { recursive: true });
if (existsSync(destination)) {
  if (!statSync(destination).isDirectory()) {
    throw new Error(`existing artifact family at ${destination} is not a directory`);
  }
  compare(destination);
  process.exit(0);
}
const temporary = `${destination}.stage-${process.pid}-${Date.now()}`;
mkdirSync(temporary);
try {
  for (const name of expectedNames) {
    writeFileSync(join(temporary, name), expected.get(name), { flag: "wx" });
  }
  try {
    renameSync(temporary, destination);
  } catch (error) {
    if (!existsSync(destination)) throw error;
    compare(destination);
  }
} finally {
  if (existsSync(temporary)) rmSync(temporary, { recursive: true, force: true });
}
' "$source" "$destination" "$selected"
}

stage_dispatch_binding() {
  local destination="$1"
  node --input-type=module - \
    "$dispatch_policy_path" "$lifecycle" "$phase" "$selected_roster" \
    "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" <<'NODE' |
import { readFileSync, statSync } from "node:fs";
import { pathToFileURL } from "node:url";

const [
  policyPath,
  lifecycle,
  phase,
  rosterText,
  helperPath,
] = process.argv.slice(2);
const MAX_POLICY_BYTES = 1024 * 1024;
const readBounded = (path, label) => {
  const stat = statSync(path);
  if (!stat.isFile()) throw new Error(`${label} is not a regular file`);
  if (stat.size > MAX_POLICY_BYTES) {
    throw new Error(`${label} exceeds ${MAX_POLICY_BYTES} bytes`);
  }
  const bytes = readFileSync(path);
  if (bytes.length > MAX_POLICY_BYTES) {
    throw new Error(`${label} exceeds ${MAX_POLICY_BYTES} bytes`);
  }
  return bytes;
};
const policy = JSON.parse(
  readBounded(policyPath, "dispatch policy").toString("utf8"),
);
const { readSelectionTable, stableStringify } =
  await import(pathToFileURL(helperPath).href);
const table = readSelectionTable();
const allSeats = [...table.mandatory_seats, ...table.optional_seats];
const exactKeys = (value, expected, label) => {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join("\0") !== [...expected].sort().join("\0")
  ) {
    throw new Error(`${label} has an unexpected shape`);
  }
};
exactKeys(policy, ["artifact_kind", "schema_version", "seats"], "dispatch policy");
if (
  policy.artifact_kind !== "d2b-panel/dispatch-policy" ||
  policy.schema_version !== 1
) {
  throw new Error("dispatch policy has an unsupported artifact kind or schema");
}
exactKeys(policy.seats, allSeats, "dispatch policy seats");
const bindingKeys = [
  "agent_type",
  "model",
  "reasoning_effort",
  "context_tier",
  "communication",
];
for (const seat of allSeats) {
  const binding = policy.seats[seat];
  exactKeys(binding, bindingKeys, `dispatch policy seat ${seat}`);
  for (const key of bindingKeys) {
    if (typeof binding[key] !== "string" || binding[key].trim() === "") {
      throw new Error(`dispatch policy seat ${seat} has no ${key}`);
    }
  }
  if (binding.agent_type !== `panel-${seat}`) {
    throw new Error(
      `dispatch policy seat ${seat} agent_type must be panel-${seat}`,
    );
  }
  if (
    binding.model !== "gpt-5.6-sol" ||
    binding.reasoning_effort !== "xhigh" ||
    binding.context_tier !== "default" ||
    binding.communication !== "caveman-full-optional"
  ) {
    throw new Error(
      `dispatch policy seat ${seat} disagrees with the current panel binding`,
    );
  }
}
const roster = rosterText.split(",").filter(Boolean);
if (
  roster.length === 0 ||
  new Set(roster).size !== roster.length ||
  roster.some((seat) => !allSeats.includes(seat))
) {
  throw new Error("selected roster cannot be projected into dispatch binding");
}
process.stdout.write(
  stableStringify({
    artifact_kind: "d2b-panel/dispatch-binding",
    schema_version: 1,
    lifecycle_id: lifecycle,
    phase,
    roster,
    bindings: Object.fromEntries(
      roster.map((seat) => [seat, policy.seats[seat]]),
    ),
  }),
);
NODE
    publish_stdin_no_replace "$destination"
}

stage() {
  local dest="$1"
  shift
  "$@" | publish_stdin_no_replace "$dest"
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
const [selectionPath, helperPath] = process.argv.slice(1);
const { candidateFromSelection, readSelection, stableStringify } =
  await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
process.stdout.write(stableStringify(candidateFromSelection(selection)));
' "$staged_selection_path" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" |
  publish_stdin_no_replace "$staged_candidate_path"
fi
node --input-type=module -e '
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const [selectionPath, candidatePath, helperPath] = process.argv.slice(1);
const { readSelection, validateSelectionCandidate } =
  await import(pathToFileURL(helperPath).href);
const selection = readSelection(selectionPath);
validateSelectionCandidate(
  selection,
  JSON.parse(readFileSync(candidatePath, "utf8")),
);
' "$staged_selection_path" "$staged_candidate_path" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs"

if [ "$reuse_existing" != true ] || [ "$existing_completion_schema" = "3" ]; then
  if ! stage_dispatch_binding "$staged_dispatch_binding_path"; then
    echo "could not materialize the roster-projected dispatch binding" >&2
    exit 2
  fi
fi

materialize_exact "$evidence_path" "$out/evidence.md" \
  "finalized validation evidence"
IFS=$'\t' read -r evidence_sha evidence_bytes \
  <<<"$(secure_digest_size "$out/evidence.md")"

if [ "$reuse_existing" != true ]; then
  for seat in "${panel_seats[@]}"; do
    materialize_exact \
      "$root/.github/agents/panel-$seat.agent.md" \
      "$staged_agent_definitions_dir/panel-$seat.agent.md" \
      "panel agent definition for $seat"
  done
fi

if [ -n "$discovery_request_path" ]; then
  if ! node --input-type=module -e '
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const [source, evidenceSha256, evidenceBytesText, helperPath] =
  process.argv.slice(1);
const { stableStringify } =
  await import(pathToFileURL(helperPath).href);
const request = JSON.parse(readFileSync(source, "utf8"));
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
process.stdout.write(stableStringify(request));
' "$discovery_request_path" "$evidence_sha" "$evidence_bytes" \
    "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" |
    publish_stdin_no_replace "$staged_discovery_request_path"
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

  publish_directory "$verification_dir" "$staged_verification_dir" "$selected_roster"
fi

if [ "$phase" = "discovery" ]; then
  if ! node --input-type=module -e '
import { readFileSync } from "node:fs";
const [requestPath, evidenceSha256, evidenceBytesText] = process.argv.slice(1);
const request = JSON.parse(readFileSync(requestPath, "utf8"));
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
' "$staged_discovery_request_path" "$evidence_sha" "$evidence_bytes"
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
import { readdirSync, readFileSync, statSync } from "node:fs";
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
const { validateStagedRoundArtifacts } =
  await import(pathToFileURL(helperPath).href);
const readJson = (path, label = "staged panel artifact") =>
  JSON.parse(readFileSync(path, "utf8"));
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
  const entries = readdirSync(verificationDir)
    .sort()
    .map((name) => ({
      name,
      bytes: readFileSync(`${verificationDir}/${name}`),
      regular: statSync(`${verificationDir}/${name}`).isFile(),
    }));
  const expectedNames = selection.roster.map((seat) => `${seat}.json`).sort();
  const actualNames = entries.map((entry) => entry.name);
  if (
    entries.length !== expectedNames.length ||
    actualNames.some((name, index) => name !== expectedNames[index]) ||
    entries.some((entry) => !entry.regular)
  ) {
    throw new Error(
      `staged verification request directory is incomplete or has extra entries; ` +
      `expected [${expectedNames.join(", ")}], found [${actualNames.join(", ")}]`,
    );
  }
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
const [round, lifecycle, selectionPath, base, previousTip, tip, phase,
  selectionSha, deltaSha, fullSha, helperPath] = process.argv.slice(1);
const { stableStringify } = await import(pathToFileURL(helperPath).href);
process.stdout.write(stableStringify({
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
}));
' "$round" "$lifecycle" "$display_staged_selection_path" "$base_sha" \
  "$prev_sha" "$tip" "$phase" "$selection_sha256" "$delta_sha" "$full_sha" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" |
  publish_stdin_no_replace "$out/address.json"

for seat in "${panel_seats[@]}"; do
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
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const [ledgerPath, helperPath] = process.argv.slice(2);
const ledger = JSON.parse(readFileSync(ledgerPath, "utf8"));
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
blocking. `late_findings` is always present and is either `[]` or an array of
objects with exactly `severity`, `introduced_regression`, `previously_missed`,
`category`,
`source_id`, `source_ordinal`, `seat`, `attribution`, `raw_text`,
`description`, `impact`, and `recommendation`. For a non-empty entry,
`introduced_regression` is `true` or `previously_missed` is `true`, and
`severity` is `critical`, `high`, `medium`, or `low`; `category` is one of
`correctness`, `security`, `data-loss`, or `reliability`. Each non-empty
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
- Roster-projected dispatch binding: \`$display_staged_dispatch_binding_path\`
- Bound panel agent definition: \`$display_staged_agent_definitions_dir/panel-<your-seat>.agent.md\`
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

This is the $phase phase. Read and follow the complete review request at $display_out/review-request.md. Use view to read every artifact it names, including the bound panel agent definition at $display_staged_agent_definitions_dir/panel-<your-seat>.agent.md, the staged current candidate, generated lifecycle artifacts, the delta, and your seat-specific notes. The active phase and verdict contract below are authoritative over any inactive-phase example in the agent definition. Review the delta rather than a prose summary, and return only the exact JSON object required below.

Required $phase verdict contract:

$verdict_contract
MD
} | publish_stdin_no_replace "$out/dispatch-prompt.txt"

node --input-type=module - "$out" "$existing_completion_schema" \
  "${canonical_artifacts[@]}" <<'NODE'
import { chmodSync } from "node:fs";
const [root, schema, ...relativePaths] = process.argv.slice(2);
for (const relative of relativePaths) {
  if (
    schema === "2" &&
    (relative === "dispatch-binding.json" ||
      relative.startsWith("agent-definitions/"))
  ) {
    continue;
  }
  chmodSync(`${root}/${relative}`, 0o444);
}
NODE

if [ "$reuse_existing" = true ] && [ "$existing_completion_schema" = "2" ]; then
  :
elif ! node --input-type=module -e '
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const [path, round, lifecycle, base, previousTip, tip, phase, selectionSha,
  deltaSha, fullSha, helperPath, ...artifactPaths] = process.argv.slice(1);
const { stableStringify } = await import(pathToFileURL(helperPath).href);
const artifactSha256 = {};
const artifactBytes = {};
for (const relative of artifactPaths) {
  const bytes = readFileSync(`${path}/${relative}`);
  artifactSha256[relative] = createHash("sha256").update(bytes).digest("hex");
  artifactBytes[relative] = bytes.length;
}
process.stdout.write(stableStringify({
  artifact_kind: "d2b-panel/stage-completion",
  schema_version: 3,
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
  artifact_sha256: artifactSha256,
  artifact_bytes: artifactBytes,
}));
' "$out" "$round" "$lifecycle" "$base_sha" "$prev_sha" "$tip" \
  "$phase" "$selection_sha256" "$delta_sha" "$full_sha" \
  "$root/.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs" \
  "${canonical_artifacts[@]}" |
  publish_stdin_no_replace "$completion_marker"
then
  echo "could not publish the completion marker" >&2
  exit 2
fi
node --input-type=module -e '
import { chmodSync } from "node:fs";
chmodSync(process.argv[1], 0o444);
' "$completion_marker"

echo "staged $display_out"
echo "  tip          $tip"
echo "  delta        $prev_sha..$tip  ($delta_sha)"
echo "  full         $base_sha..$tip  ($full_sha)"
echo
echo "Finalized evidence and reviewer notes are byte-bound by $display_completion_marker."
echo "Dispatch every seat with the exact contents of $display_out/dispatch-prompt.txt."
