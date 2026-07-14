#!/usr/bin/env bash
# Consolidated generated-artifact drift gate.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

d2b_activate_rust_toolchain_path || true
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"

cd "$ROOT"

if [ -z "${D2B_DRIFT_CHECK_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "drift-check: neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export D2B_DRIFT_CHECK_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

generator_root="$ROOT"
workspace_target_dir="${CARGO_TARGET_DIR:-$(d2b_cargo_target_dir workspace)}"
if [ -n "${D2B_VALIDATION_OUTPUT_DIR:-}" ]; then
  generator_root=$(d2b_mktemp ".d2b-drift-source.XXXXXX")
  git clone --no-hardlinks --quiet -- "$ROOT" "$generator_root"
  git -C "$generator_root" checkout --detach --quiet "$(git -C "$ROOT" rev-parse HEAD)"
  workspace_target_dir="$D2B_VALIDATION_OUTPUT_DIR/drift-cargo-target"
fi
xtask_bin="$workspace_target_dir/debug/xtask"
(
  cd "$generator_root/packages"
  # Always ask Cargo to refresh xtask in the selected target dir. Cargo reuses
  # cached artifacts when fresh, but this prevents an old repo-local
  # packages/target/debug/xtask from masking generated schema/docs drift.
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo build -q --manifest-path "$generator_root/packages/Cargo.toml" -p xtask --bin xtask
)

run_xtask() {
  local subcommand="$1"
  log "--> drift-check: cargo xtask $subcommand"
  (
    cd "$generator_root/packages"
    "$xtask_bin" "$subcommand"
  )
}

run_xtask gen-schemas
run_xtask gen-error-codes
run_xtask gen-daemon-api
run_xtask gen-cli-shell-artifacts
run_xtask gen-cli-schemas
run_xtask gen-guest-proto
run_xtask gen-guest-ttrpc
run_xtask gen-ttrpc-api-fit-spike
run_xtask gen-v2-services

drift_paths=(
  docs/reference/schemas/
  docs/reference/error-codes.md
  docs/reference/daemon-api.md
  docs/manpages/
  docs/completions/
  docs/reference/cli-output/
  packages/d2b-contracts/src/generated
  packages/d2b-contracts/src/generated_v2_services
  packages/d2b-guestd/src/generated
  packages/d2b-ttrpc-api-fit-spike/src/generated
  docs/reference/v2-services.json
  docs/reference/v2-services-schema.json
)

if git -C "$generator_root" --no-pager diff --exit-code -- "${drift_paths[@]}" >/dev/null; then
  ok "drift-check: generated artifacts match committed outputs"
else
  git -C "$generator_root" --no-pager diff -- "${drift_paths[@]}" | head -120 >&2 || true
  fail "drift-check: generated artifacts drifted; rerun tests/unit/gates/drift-check.sh and commit the generated outputs"
fi

grep -Fq '<!-- BEGIN AUTO-GENERATED: error-table -->' "$generator_root/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table begin marker"
grep -Fq '<!-- END AUTO-GENERATED: error-table -->' "$generator_root/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table end marker"
# shellcheck disable=SC2016
grep -Eq '^\| <a id="[a-z0-9-]+"></a>`#[a-z0-9-]+` \| `[a-z0-9-]+` \| `[0-9]+` \|' "$generator_root/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing generated per-kind rows"

ok "drift-check: generated error-code table markers and rows are present"
