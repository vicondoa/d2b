#!/usr/bin/env bash
# U6 candidate-bound daily-driver cutover lane.
#
# This is the destructive manual lane. It validates the frozen candidate and
# recovery evidence through the production delivery and host-cutover
# validators, then stops at CutoverSucceeded. U7 owns merge, post-merge seal,
# and the separately consented phase-10 finalization.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

log() {
  printf '[cutover-real-host] %s\n' "$*" >&2
}

refuse() {
  log "REFUSED: $*"
  return 78
}

require_value() {
  local label="$1"
  local value="$2"
  if [[ -z "$value" ]]; then
    refuse "missing required $label"
    exit 78
  fi
}

require_path() {
  local label="$1"
  local path="$2"
  case "$path" in
    /*) ;;
    *)
      refuse "$label must be an exact absolute path"
      exit 78
      ;;
  esac
  if [[ ! -e "$path" ]]; then
    refuse "required $label is unavailable"
    exit 78
  fi
}

run_validator() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    return 0
  fi
  refuse "validation failed before mutation: $label; raw paths and identities are intentionally suppressed"
  exit 78
}

run_json_validator() {
  local label="$1"
  local predicate="$2"
  shift 2
  local output
  if ! output=$("$@" 2>&1); then
    refuse "validation failed before mutation: $label; raw paths and identities are intentionally suppressed"
    exit 78
  fi
  local predicate_result
  if ! predicate_result=$(jq -r "$predicate" <<<"$output" 2>/dev/null); then
    refuse "validation returned no valid public cutover state: $label"
    exit 78
  fi
  if [[ "$predicate_result" != "true" ]]; then
    refuse "validation returned no valid public cutover state: $label"
    exit 78
  fi
}

# The live gate is an explicit operator decision, not a default. Refuse before
# building the heavy-lane helper when it is absent.
if [[ "${D2B_LIVE:-0}" != "1" ]]; then
  refuse "set D2B_LIVE=1 for the manual daily-driver lane"
  exit 78
fi

# --- heavy-gate sole-use semaphore (ADR 0046) ------------------------------
# The helper proves that this process owns a protected slot and re-execs once
# through the public heavy gate when it does not. Do not replace it with an
# environment-marker check or a second lock.
# shellcheck source=tests/tools/heavy-gate-reexec.sh
. "$ROOT/tests/tools/heavy-gate-reexec.sh"
d2b_heavy_gate_reexec "$ROOT" "$0" "$@"

require_value "delivery state directory" "${D2B_LIVE_STATE_DIR:-}"
require_value "candidate directory" "${D2B_LIVE_CANDIDATE_DIR:-}"
require_value "repository id" "${D2B_LIVE_REPO_ID:-}"
require_value "repository root" "${D2B_LIVE_REPO_ROOT:-}"
require_value "operation id" "${D2B_LIVE_OPERATION_ID:-}"
require_value "candidate id" "${D2B_LIVE_CANDIDATE_ID:-}"
require_value "revision-plan id" "${D2B_LIVE_REVISION_PLAN_ID:-}"
require_value "preview digest" "${D2B_LIVE_PREVIEW_DIGEST:-}"
require_value "recovery digest" "${D2B_LIVE_RECOVERY_DIGEST:-}"
require_value "operator id" "${D2B_LIVE_OPERATOR_ID:-}"
require_value "apply consent digest" "${D2B_LIVE_CONSENT_DIGEST:-}"
require_value "host digest" "${D2B_LIVE_HOST_DIGEST:-}"

require_value "release commit id" "${D2B_LIVE_COMMIT_OID:-}"
require_value "release tree id" "${D2B_LIVE_TREE_OID:-}"
require_value "closure path digest" "${D2B_LIVE_CLOSURE_PATH_DIGEST:-}"
require_value "bundle generation" "${D2B_LIVE_BUNDLE_GENERATION:-}"
require_value "host identity digest" "${D2B_LIVE_HOST_IDENTITY_DIGEST:-}"
require_value "operator subject digest" "${D2B_LIVE_OPERATOR_SUBJECT_DIGEST:-}"
require_value "restore-instruction digest" "${D2B_LIVE_RESTORE_INSTRUCTIONS_DIGEST:-}"
require_value "recovery locator digest" "${D2B_LIVE_RECOVERY_LOCATOR_DIGEST:-}"
require_value "recovery verifier command" "${D2B_LIVE_RECOVERY_COMMAND:-}"
require_value "required recovery TTL" "${D2B_LIVE_REQUIRED_REMAINING_TTL_SECONDS:-}"
require_value "recovery verifier time" "${D2B_LIVE_VERIFIER_NOW_UNIX:-}"

require_path "candidate directory" "$D2B_LIVE_CANDIDATE_DIR"
require_path "candidate snapshot" "${D2B_LIVE_SNAPSHOT:-}"
require_path "candidate seal" "${D2B_LIVE_SEAL:-}"
require_path "qualified recovery attestation" "${D2B_LIVE_RECOVERY_ATTESTATION:-}"
require_path "cutover recovery evidence" "${D2B_LIVE_CUTOVER_RECOVERY:-}"
require_path "exact apply consent" "${D2B_LIVE_CONSENT:-}"
require_path "typed host-generation handoff" "${D2B_LIVE_HANDOFF:-}"
require_path "all-Zone verification evidence" "${D2B_LIVE_VERIFICATION:-}"

if ! system_artifact_id=$(jq -r '.intent.systemArtifactId // empty' "$D2B_LIVE_HANDOFF"); then
  refuse "typed handoff is not valid public JSON"
  exit 78
fi
if [[ -z "$system_artifact_id" ]]; then
  refuse "typed handoff has no system artifact identity"
  exit 78
fi
source_system_artifact_id="${D2B_LIVE_SOURCE_SYSTEM_ARTIFACT_ID:-}"
if [[ -z "$source_system_artifact_id" ]]; then
  refuse "preserved source artifact identity is required"
  exit 78
fi

repo_binding="$D2B_LIVE_REPO_ID=$D2B_LIVE_REPO_ROOT"

# U5 validates the strict external recovery attestation, writes only its
# digest-addressed evidence record, and rejects candidate/evidence drift.
run_validator "qualified recovery evidence" \
  cargo run --quiet --locked --manifest-path "$ROOT/Cargo.toml" -p xtask -- \
    delivery wave recovery-import \
    --snapshot "$D2B_LIVE_SNAPSHOT" \
    --attestation "$D2B_LIVE_RECOVERY_ATTESTATION" \
    --repo "$repo_binding" \
    --candidate-id "$D2B_LIVE_CANDIDATE_ID" \
    --commit-oid "$D2B_LIVE_COMMIT_OID" \
    --tree-oid "$D2B_LIVE_TREE_OID" \
    --closure-store-path-sha256 "$D2B_LIVE_CLOSURE_PATH_DIGEST" \
    --bundle-generation "$D2B_LIVE_BUNDLE_GENERATION" \
    --preview-sha256 "$D2B_LIVE_PREVIEW_DIGEST" \
    --host-identity-sha256 "$D2B_LIVE_HOST_IDENTITY_DIGEST" \
    --operator-subject-sha256 "$D2B_LIVE_OPERATOR_SUBJECT_DIGEST" \
    --restore-instructions-sha256 "$D2B_LIVE_RESTORE_INSTRUCTIONS_DIGEST" \
    --recovery-point-locator-sha256 "$D2B_LIVE_RECOVERY_LOCATOR_DIGEST" \
    --required-remaining-ttl-seconds "$D2B_LIVE_REQUIRED_REMAINING_TTL_SECONDS" \
    --verifier-now-unix "$D2B_LIVE_VERIFIER_NOW_UNIX" \
    --command "$D2B_LIVE_RECOVERY_COMMAND" \
    --state-dir "$D2B_LIVE_STATE_DIR"

run_validator "candidate seal" \
  cargo run --quiet --locked --manifest-path "$ROOT/Cargo.toml" -p xtask -- \
    delivery wave seal \
    --snapshot "$D2B_LIVE_SNAPSHOT" \
    --repo "$repo_binding" \
    --state-dir "$D2B_LIVE_STATE_DIR"

run_validator "current merge eligibility" \
  cargo run --quiet --locked --manifest-path "$ROOT/Cargo.toml" -p xtask -- \
    delivery wave merge-eligibility \
    --seal "$D2B_LIVE_SEAL" \
    --repo "$repo_binding" \
    --state-dir "$D2B_LIVE_STATE_DIR"

# U3 constructs the authoritative host-wide, all-Zone preview. Its typed
# response is deliberately suppressed here: it contains only redaction-safe
# fields, but this lane never writes candidate paths or opaque ids to logs.
run_validator "host-wide cutover preview" \
  d2b host cutover preview \
    --operation-id "$D2B_LIVE_OPERATION_ID" \
    --candidate-id "$D2B_LIVE_CANDIDATE_ID" \
    --revision-plan-id "$D2B_LIVE_REVISION_PLAN_ID" \
    --system-artifact-id "$system_artifact_id" \
    --source-system-artifact-id "$source_system_artifact_id" \
    --json

# This is the first host mutation. U3/U4 revalidate the candidate, preview,
# exact consent, qualified recovery, ownership, and typed handoff before the
# broker admits the out-of-band runner.
run_json_validator "candidate-bound cutover apply" \
  'type == "object" and .ok == true and .operation == "apply" and ((.mutationAccepted == true) or (.mutationAccepted == false and .summary == "cutover apply response lost; runner state observed")) and (.operationId | type == "string") and (.state == "applying" or .state == "cutover-succeeded") and (.phase | type == "number") and .phase >= 5' \
  d2b host cutover apply \
    --operation-id "$D2B_LIVE_OPERATION_ID" \
    --candidate-id "$D2B_LIVE_CANDIDATE_ID" \
    --revision-plan-id "$D2B_LIVE_REVISION_PLAN_ID" \
    --system-artifact-id "$system_artifact_id" \
    --source-system-artifact-id "$source_system_artifact_id" \
    --preview-digest "$D2B_LIVE_PREVIEW_DIGEST" \
    --recovery-digest "$D2B_LIVE_RECOVERY_DIGEST" \
    --operator-id "$D2B_LIVE_OPERATOR_ID" \
    --consent-digest "$D2B_LIVE_CONSENT_DIGEST" \
    --consent-file "$D2B_LIVE_CONSENT" \
    --recovery-attestation-file "$D2B_LIVE_CUTOVER_RECOVERY" \
    --host-digest "$D2B_LIVE_HOST_DIGEST" \
    --handoff-file "$D2B_LIVE_HANDOFF" \
    --json

run_json_validator "runner status after drain and handoff" \
  'type == "object" and .ok == true and .operation == "status" and .mutationAccepted == false and (.operationId | type == "string") and (.state == "applying" or .state == "cutover-succeeded") and (.phase | type == "number") and .phase >= 5' \
  d2b host cutover status \
    --operation-id "$D2B_LIVE_OPERATION_ID" \
    --json

# The production runner owns the remaining typed effects and all phase
# transitions. Verification must observe every configured Zone, preserved
# identity digest, retained source, current candidate, and durable audit chain.
run_json_validator "all-Zone cutover verification" \
  'type == "object" and .ok == true and .operation == "verify" and .mutationAccepted == true and (.operationId | type == "string") and .state == "cutover-succeeded" and .phase == 9' \
  d2b host cutover verify \
    --operation-id "$D2B_LIVE_OPERATION_ID" \
    --verification-file "$D2B_LIVE_VERIFICATION" \
    --json

run_json_validator "post-cutover doctor" \
  'type == "object" and .ok == true and .operation == "doctor" and .mutationAccepted == false and (.operationId | type == "string") and .state == "cutover-succeeded" and .phase == 9' \
  d2b host cutover doctor \
    --operation-id "$D2B_LIVE_OPERATION_ID" \
    --json

log "CutoverSucceeded observed; finalization is intentionally not invoked"
log "raw paths and identities are intentionally suppressed"
