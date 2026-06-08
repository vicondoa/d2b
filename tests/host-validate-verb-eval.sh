#!/usr/bin/env bash
# tests/host-validate-verb-eval.sh— Layer-1 gate.
#
# Asserts the operator-facing `nixling host validate` composite
# preflight verb:
#
#   1. The Rust CLI exposes a `host validate` subcommand with both
#      `--dry-run` and `--apply` mutation flags (and refuses without
#      either, per the --apply-or-dry-run-required envelope).
#   2. The wave vocabulary in the verb's catalog
#      (`packages/nixling/src/host_validate.rs::WAVE_CATALOG`) is
#      byte-identical to the readiness vocabulary in
#      `nixos-modules/options-daemon.nix:readinessWaveSpecs`. The
#      auto-flip gate
#      (`options-daemon.nix:validationEvidencePresent`) consumes these
#      evidence records; drift between the two surfaces silently
#      breaks the gate.
#   3. `--dry-run --json` reports every wave with status + per-
#      validator presence map; no evidence is written; exit 0.
#   4. `--apply --wave p1` (or any wave with all validators present)
#      writes `/var/lib/nixling/validated/<wave>.json` with the
#      canonical schema fields:
#         - wave (string equal to <wave>)
#         - timestamp (non-empty string)
#         - operatorSignature (non-empty string)
#      Exit 0.
#   5. `--apply --wave <name>` against an empty scripts dir refuses
#      with exit 78 (`Missing` → "evidence NOT written").
#   6. `--apply --wave bogus-wave` returns the typed
#      `unknown-wave` envelope (exit 78).
#
# See docs/reference/host-validate.md for the full verb contract.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/host-validate-verb-eval.sh"

PASS=0
FAIL=0
pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# ---------------------------------------------------------------
# Provision a cargo + rustc toolchain so we can build the CLI in
# the same shape as the rest of the static-fast gates.
# ---------------------------------------------------------------

if ! command -v cargo >/dev/null 2>&1; then
  log "  setup: provisioning cargo + rustc via nix shell"
  rust_path=$(nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy \
    nixpkgs#gcc nixpkgs#sccache \
    --command bash -lc 'printf %s "$PATH"')
  export PATH="$rust_path:$PATH"
fi

if ! command -v cargo >/dev/null 2>&1; then
  fail_check "setup: cargo still not on PATH after nix shell provisioning"
  log "==> $FAIL failure(s)"
  exit 1
fi

# ---------------------------------------------------------------
# Build the nixling CLI binary used by the gate.
# Honour the AGENTS.md "Disk hygiene contract" by reusing the
# shared cargo target dir; honour the integrator instruction to
# unset RUSTC_WRAPPER for these dev builds.
# ---------------------------------------------------------------

log "  setup: building packages/nixling (release) with RUSTC_WRAPPER unset"
(
  cd "$ROOT/packages"
  RUSTC_WRAPPER="" cargo build --quiet -p nixling --release
) || {
  fail_check "setup: cargo build -p nixling --release"
  log "==> $FAIL failure(s)"
  exit 1
}

BIN=""
for candidate in \
  "$ROOT/packages/target/release/nixling" \
  "${CARGO_TARGET_DIR:-}/release/nixling" \
  "/home/paydro/.cache/nixling-cargo-target/release/nixling"
do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then
    BIN="$candidate"
    break
  fi
done
if [ -z "$BIN" ]; then
  fail_check "setup: could not locate the freshly-built nixling binary"
  log "==> $FAIL failure(s)"
  exit 1
fi
log "  setup: using BIN=$BIN"

# ---------------------------------------------------------------
# Wave-vocabulary parity between the Rust catalog and
# nixos-modules/options-daemon.nix:readinessWaveSpecs. This is the
# load-bearing contract check.
# ---------------------------------------------------------------

CATALOG_FILE="$ROOT/packages/nixling/src/host_validate.rs"
OPTIONS_FILE="$ROOT/nixos-modules/options-daemon.nix"

extract_catalog_waves() {
  # WAVE_CATALOG entries are of the form `wave: "name",` on their
  # own line. Order-preserving.
  awk '/^pub const WAVE_CATALOG/,/^];/' "$CATALOG_FILE" \
    | sed -n 's/^[[:space:]]*wave:[[:space:]]*"\([^"]*\)".*/\1/p'
}

extract_options_waves() {
  # readinessWaveSpecs lives in options-daemon.nix at indent level
  # 2 ("  readinessWaveSpecs = {"); its wave entries sit at indent
  # level 4 ("    <wave> = {"). Anchor on that exact indentation so
  # we don't pick up the binding line itself, nor inner field lines.
  sed -n 's/^    \([A-Za-z][A-Za-z0-9]*\) = {[[:space:]]*$/\1/p' "$OPTIONS_FILE"
}

catalog_waves=$(extract_catalog_waves)
options_waves=$(extract_options_waves)

if [ -z "$catalog_waves" ]; then
  fail_check "catalog parity: failed to extract WAVE_CATALOG entries from $CATALOG_FILE"
elif [ -z "$options_waves" ]; then
  fail_check "catalog parity: failed to extract readinessWaveSpecs from $OPTIONS_FILE"
elif [ "$catalog_waves" = "$options_waves" ]; then
  pass_check "catalog parity: WAVE_CATALOG order + names match readinessWaveSpecs"
else
  fail_check "catalog parity: WAVE_CATALOG drift vs readinessWaveSpecs"
  log "  catalog:"
  printf '%s\n' "$catalog_waves" | sed 's/^/    /' >&2
  log "  options:"
  printf '%s\n' "$options_waves" | sed 's/^/    /' >&2
fi

# ---------------------------------------------------------------
# Scratch fixtures.
# ---------------------------------------------------------------

SCRATCH=$(nl_mktemp -d -t nixling-host-validate-eval)
trap 'rm -rf "$SCRATCH"' EXIT
SCRIPTS_FULL="$SCRATCH/scripts-full"
SCRIPTS_EMPTY="$SCRATCH/scripts-empty"
EVIDENCE="$SCRATCH/evidence"
mkdir -p "$SCRIPTS_FULL" "$SCRIPTS_EMPTY" "$EVIDENCE"

# Stage every validator basename declared in the Rust catalog so
# every wave (other than p6/p7) reports `ready`. Extract the names
# directly from WAVE_CATALOG so the test stays in sync.
catalog_scripts=$(awk '/^pub const WAVE_CATALOG/,/^];/' "$CATALOG_FILE" \
  | sed -n 's/^[[:space:]]*"\([a-zA-Z0-9_-]*\.sh\)".*/\1/p' \
  | sort -u)
if [ -z "$catalog_scripts" ]; then
  fail_check "validator fixtures: failed to extract validator basenames from WAVE_CATALOG"
fi
while IFS= read -r name; do
  [ -n "$name" ] || continue
  : > "$SCRIPTS_FULL/$name"
done <<<"$catalog_scripts"
pass_check "validator fixtures: staged $(printf '%s\n' "$catalog_scripts" | wc -l) validator stubs"

# ---------------------------------------------------------------
# Refusal when neither --dry-run nor --apply is given.
# ---------------------------------------------------------------

set +e
refusal_out=$(NIXLING_VALIDATE_SCRIPTS_DIR="$SCRIPTS_FULL" \
  NIXLING_VALIDATE_EVIDENCE_DIR="$EVIDENCE" \
  "$BIN" host validate --json 2>/dev/null)
refusal_rc=$?
set -e
if [ "$refusal_rc" = "78" ] \
  && printf '%s' "$refusal_out" | grep -q '"code": "--apply-or-dry-run-required"'; then
  pass_check "mutation flag validation: missing mode refused with exit 78 + typed envelope"
else
  fail_check "mutation flag validation: expected exit 78 + --apply-or-dry-run-required envelope; got rc=$refusal_rc"
  log "    body: $(printf '%s' "$refusal_out" | head -c 200)"
fi

# ---------------------------------------------------------------
# --dry-run --json reports every wave; no evidence written.
# ---------------------------------------------------------------

set +e
dry_out=$(NIXLING_VALIDATE_SCRIPTS_DIR="$SCRIPTS_FULL" \
  NIXLING_VALIDATE_EVIDENCE_DIR="$EVIDENCE" \
  "$BIN" host validate --dry-run --json 2>/dev/null)
dry_rc=$?
set -e
if [ "$dry_rc" = "0" ]; then
  pass_check "dry-run: exit 0"
else
  fail_check "dry-run: expected exit 0, got $dry_rc"
fi

# Count waves in the JSON output (one `"wave":` per entry inside the
# `waves` array; tally those that come AFTER the `"waves":` key).
wave_count=$(printf '%s' "$dry_out" \
  | awk '/"waves":/{found=1; next} found && /"wave":/{n++} END{print n+0}')
expected_count=$(printf '%s\n' "$catalog_waves" | wc -l | tr -d ' ')
if [ "$wave_count" = "$expected_count" ]; then
  pass_check "dry-run: reports $wave_count waves (matches catalog)"
else
  fail_check "dry-run: reported $wave_count waves; expected $expected_count"
fi

if [ ! -e "$EVIDENCE/p1.json" ] && [ ! -e "$EVIDENCE/p0.json" ]; then
  pass_check "dry-run: wrote no evidence files"
else
  fail_check "dry-run: unexpectedly wrote evidence files under $EVIDENCE"
fi

# ---------------------------------------------------------------
# --apply --wave p1 writes the canonical evidence record.
# ---------------------------------------------------------------

rm -rf "$EVIDENCE" && mkdir -p "$EVIDENCE"
set +e
apply_out=$(NIXLING_VALIDATE_SCRIPTS_DIR="$SCRIPTS_FULL" \
  NIXLING_VALIDATE_EVIDENCE_DIR="$EVIDENCE" \
  "$BIN" host validate --apply --wave p1 --json 2>/dev/null)
apply_rc=$?
set -e
if [ "$apply_rc" = "0" ]; then
  pass_check "apply selected wave: --apply --wave p1 exit 0"
else
  fail_check "apply selected wave: --apply --wave p1 expected exit 0, got $apply_rc"
  log "    body: $(printf '%s' "$apply_out" | head -c 400)"
fi

if [ -f "$EVIDENCE/p1.json" ]; then
  pass_check "apply selected wave: evidence file $EVIDENCE/p1.json exists"
else
  fail_check "apply selected wave: evidence file $EVIDENCE/p1.json missing"
fi

evidence_body=$(cat "$EVIDENCE/p1.json" 2>/dev/null || true)
check_field() {
  local field="$1" expected_substr="$2"
  if printf '%s' "$evidence_body" \
    | grep -qE "\"${field}\"[[:space:]]*:[[:space:]]*\"${expected_substr}"; then
    pass_check "apply selected wave: evidence carries $field"
  else
    fail_check "apply selected wave: evidence missing/malformed $field"
    log "    body: $evidence_body"
  fi
}
check_field "wave" "p1\""
check_field "timestamp" "[0-9][0-9][0-9][0-9]-"
check_field "operatorSignature" "sha256:"

# Negative: only p1.json should exist, all other waves are `skipped`.
other_files=$(find "$EVIDENCE" -maxdepth 1 -type f -name '*.json' ! -name 'p1.json' | wc -l | tr -d ' ')
if [ "$other_files" = "0" ]; then
  pass_check "apply selected wave: --wave filter constrained evidence write to p1.json"
else
  fail_check "apply selected wave: --wave filter leaked $other_files unrelated evidence files"
fi

# ---------------------------------------------------------------
# --apply with missing validators refuses (exit 78,
# evidence NOT written).
# ---------------------------------------------------------------

rm -rf "$EVIDENCE" && mkdir -p "$EVIDENCE"
set +e
miss_out=$(NIXLING_VALIDATE_SCRIPTS_DIR="$SCRIPTS_EMPTY" \
  NIXLING_VALIDATE_EVIDENCE_DIR="$EVIDENCE" \
  "$BIN" host validate --apply --wave p1 --json 2>/dev/null)
miss_rc=$?
set -e
if [ "$miss_rc" = "78" ]; then
  pass_check "missing validators: apply exits 78"
else
  fail_check "missing validators: apply expected exit 78, got $miss_rc"
  log "    body: $(printf '%s' "$miss_out" | head -c 400)"
fi
if [ ! -e "$EVIDENCE/p1.json" ]; then
  pass_check "missing validators: no evidence file written"
else
  fail_check "missing validators: evidence file unexpectedly written"
fi

# ---------------------------------------------------------------
# --apply --wave bogus returns the unknown-wave envelope.
# ---------------------------------------------------------------

set +e
bogus_out=$(NIXLING_VALIDATE_SCRIPTS_DIR="$SCRIPTS_FULL" \
  NIXLING_VALIDATE_EVIDENCE_DIR="$EVIDENCE" \
  "$BIN" host validate --apply --wave bogus-wave --json 2>/dev/null)
bogus_rc=$?
set -e
if [ "$bogus_rc" = "78" ] \
  && printf '%s' "$bogus_out" | grep -q '"code": "unknown-wave"'; then
  pass_check "unknown wave: returned unknown-wave envelope (exit 78)"
else
  fail_check "unknown wave: expected exit 78 + unknown-wave envelope; got rc=$bogus_rc"
  log "    body: $(printf '%s' "$bogus_out" | head -c 200)"
fi

# ---------------------------------------------------------------
# Tally
# ---------------------------------------------------------------

if [ "$FAIL" -gt 0 ]; then
  log "==> $FAIL failure(s), $PASS pass(es)"
  exit 1
fi
log "==> ALL CHECKS PASSED ($PASS check(s))"
exit 0
