#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

log "==> tests/error-codes-drift.sh"
if [ -z "${NIXLING_ERROR_CODES_DRIFT_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "error-codes-drift: neither cargo nor nix is on PATH"
    exit 1
  fi
  export NIXLING_ERROR_CODES_DRIFT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi
xtask_bin=$(nl_cargo_bin_path workspace xtask)
if [ ! -x "$xtask_bin" ]; then
  (cd "$ROOT/packages" && CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" cargo build -q --manifest-path "$ROOT/packages/Cargo.toml" -p xtask --bin xtask)
fi
(cd "$ROOT/packages" && "$xtask_bin" gen-error-codes) >/dev/null

if git -C "$ROOT" --no-pager diff --exit-code -- docs/reference/error-codes.md >/dev/null; then
  ok "cargo xtask gen-error-codes is deterministic"
else
  git -C "$ROOT" --no-pager diff -- docs/reference/error-codes.md | head -120 >&2 || true
  fail "docs/reference/error-codes.md drifted after cargo xtask gen-error-codes"
fi

grep -Fq '<!-- BEGIN AUTO-GENERATED: error-table -->' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table begin marker"
grep -Fq '<!-- END AUTO-GENERATED: error-table -->' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing the generated error-table end marker"
grep -Eq '^\| <a id="[a-z0-9-]+"></a>`#[a-z0-9-]+` \| `[a-z0-9-]+` \| `[0-9]+` \|' "$ROOT/docs/reference/error-codes.md" \
  || fail "docs/reference/error-codes.md is missing generated per-kind rows"
ok "error-codes markdown contains generated per-kind rows with stable anchors"

log "==> error-codes-drift OK"
