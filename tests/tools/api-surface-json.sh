#!/usr/bin/env bash
# Build the compiler-derived public/private API census once and check snapshots.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

pin=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
  packages/d2b-api-surface/rust-toolchain.toml | head -1)
[ "$pin" = "nightly-2026-02-16" ] || {
  fail "api-surface toolchain pin drifted"
  exit 1
}

if ! command -v rustup >/dev/null 2>&1; then
  if [ -z "${D2B_API_SURFACE_RUSTUP_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    export D2B_API_SURFACE_RUSTUP_SHELL=1
    exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#rustup \
      --command bash "$0" "$@"
  fi
  fail "api-surface requires rustup for pinned nightly $pin"
  exit 1
fi

rustup toolchain install "$pin" --profile minimal

scratch=$(d2b_mktemp ".d2b-api-surface.XXXXXX")
public_dir="$scratch/public"
private_dir="$scratch/private"
mkdir -p "$public_dir" "$private_dir"
metadata="$scratch/workspace-metadata.json"
golden="$ROOT/tests/golden/api-surface"
target_root=${D2B_API_SURFACE_TARGET_DIR:-$ROOT/.scratch/rust-test-cache/api-surface-$pin}
public_target="$target_root/public"
private_target="$target_root/private"

# Delete only rendered JSON. Cargo's compiled intermediate artifacts remain
# reusable, but a stale or extra blob cannot satisfy the exact metadata census.
rm -rf "$public_target/doc" "$private_target/doc"

log "--> rustdoc JSON public workspace census ($pin)"
(
  cd "$ROOT/packages"
  env \
    RUSTC_WRAPPER= \
    RUSTC_WORKSPACE_WRAPPER= \
    CARGO_BUILD_RUSTC_WRAPPER= \
    RUSTDOCFLAGS="-Z unstable-options --output-format json" \
    cargo "+$pin" doc --locked --workspace --lib --no-deps \
      --target-dir "$public_target"
)
find "$public_target/doc" -maxdepth 1 -type f -name '*.json' -exec cp {} "$public_dir/" \;

log "--> rustdoc JSON private + hidden workspace census ($pin)"
(
  cd "$ROOT/packages"
  env \
    RUSTC_WRAPPER= \
    RUSTC_WORKSPACE_WRAPPER= \
    CARGO_BUILD_RUSTC_WRAPPER= \
    RUSTDOCFLAGS="-Z unstable-options --output-format json --document-private-items --document-hidden-items" \
    cargo "+$pin" doc --locked --workspace --lib --no-deps \
      --target-dir "$private_target"
)
find "$private_target/doc" -maxdepth 1 -type f -name '*.json' -exec cp {} "$private_dir/" \;

bash "$ROOT/tests/tools/gen-api-surface-metadata.sh" \
  "$public_dir" "$private_dir" "$metadata"
cmp -s "$metadata" "$golden/workspace-metadata.json" || {
  if [ "${D2B_API_SURFACE_UPDATE:-0}" = 1 ]; then
    cp "$metadata" "$golden/workspace-metadata.json"
  else
    # This file contains only crate identities and numeric census values. Show
    # the exact drift so CI identifies the changed crate without exposing raw
    # rustdoc JSON, source text, signatures, or runner-local paths.
    diff --unified=3 \
      --label committed-workspace-metadata.json \
      --label generated-workspace-metadata.json \
      "$golden/workspace-metadata.json" "$metadata" || true
    fail "api-surface workspace metadata census drifted"
    exit 1
  fi
}

mode=--check
[ "${D2B_API_SURFACE_UPDATE:-0}" = 1 ] && mode=--write
log "--> d2b-api-surface snapshot check"
cargo run --quiet --locked --manifest-path "$ROOT/packages/Cargo.toml" \
  -p d2b-api-surface --bin d2b-api-surface -- \
  --public-json-dir "$public_dir" \
  --private-json-dir "$private_dir" \
  --metadata "$metadata" \
  --roots "$golden/roots.json" \
  --public-api "$golden/public-api.txt" \
  --capability-api "$golden/capability-api.txt" \
  --hidden-public-api "$golden/hidden-public-api.txt" \
  --trait-impls "$golden/capability-trait-impls.txt" \
  "$mode"

ok "compiler-derived API and capability inventory"
