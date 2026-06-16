#!/usr/bin/env bash
# Consolidated generated-artifact drift gate.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
NL_LOG=${NL_LOG:-/dev/null}

# shellcheck disable=SC1091
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"

cd "$ROOT"

if [ -z "${NIXLING_DRIFT_CHECK_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "drift-check: neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_DRIFT_CHECK_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

xtask_bin=$(nl_cargo_bin_path workspace xtask)
if [ ! -x "$xtask_bin" ]; then
  (
    cd "$ROOT/packages"
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" \
      CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" \
      cargo build -q --manifest-path "$ROOT/packages/Cargo.toml" -p xtask --bin xtask
  )
fi

run_xtask() {
  local subcommand="$1"
  log "--> drift-check: cargo xtask $subcommand"
  (
    cd "$ROOT/packages"
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" "$xtask_bin" "$subcommand"
  )
}

run_xtask gen-schemas
run_xtask gen-error-codes
run_xtask gen-daemon-api
run_xtask gen-cli-shell-artifacts
run_xtask gen-guest-proto
run_xtask gen-guest-ttrpc

drift_paths=(
  docs/reference/schemas/
  docs/reference/error-codes.md
  docs/reference/daemon-api.md
  docs/manpages/
  docs/completions/
  packages/nixling-ipc/src/generated
  packages/nixling-guestd/src/generated
)

if git -C "$ROOT" --no-pager diff --exit-code -- "${drift_paths[@]}" >/dev/null; then
  ok "drift-check: generated artifacts match committed outputs"
else
  git -C "$ROOT" --no-pager diff -- "${drift_paths[@]}" | head -120 >&2 || true
  fail "drift-check: generated artifacts drifted; rerun tests/drift-check.sh and commit the generated outputs"
fi

grep -Fq '<!-- BEGIN AUTO-GENERATED: error-table -->' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table begin marker"
grep -Fq '<!-- END AUTO-GENERATED: error-table -->' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table end marker"
# shellcheck disable=SC2016
grep -Eq '^\| <a id="[a-z0-9-]+"></a>`#[a-z0-9-]+` \| `[a-z0-9-]+` \| `[0-9]+` \|' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing generated per-kind rows"

ok "drift-check: generated error-code table markers and rows are present"
