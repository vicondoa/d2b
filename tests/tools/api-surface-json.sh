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

rustup set profile minimal >/dev/null
rustup toolchain install "$pin" >/dev/null

scratch=$(d2b_mktemp ".d2b-api-surface.XXXXXX")
public_dir="$scratch/public"
private_dir="$scratch/private"
mkdir -p "$public_dir" "$private_dir"
metadata="$scratch/workspace-metadata.json"
golden="$ROOT/tests/golden/api-surface"
case "${D2B_API_SURFACE_UPDATE:-0}" in
  0|1) ;;
  *)
    fail "D2B_API_SURFACE_UPDATE must be 0 or 1" || true
    exit 2
    ;;
esac
target_root=${D2B_API_SURFACE_TARGET_DIR:-$ROOT/.scratch/rust-test-cache/api-surface-$pin}
case "$target_root" in
  /*) ;;
  *)
    fail "D2B_API_SURFACE_TARGET_DIR must be an absolute path" || true
    exit 2
    ;;
esac
# One target directory for both censuses. They differ only in RUSTDOCFLAGS,
# which fingerprints the rustdoc units and so still re-renders every workspace
# lib, but does NOT fingerprint the rustc units that compile the dependency
# graph. Separate directories therefore compiled that graph twice: measured on
# this workspace, the two trees held byte-identical dependency artifact sets
# (1879 files each, all names matching) for 2.6 GB of pure duplication. That
# cost is paid twice over, once as wall time on any cold run and once as
# footprint in the repository-wide Actions cache budget that
# .scratch/rust-test-cache is restored from.
census_target="$target_root/census"
checker_target="$target_root/checker"

# Reclaim the pre-merge layout. A restored cache still carries these, and left
# alone they would keep charging the cache budget for a tree nothing reads.
rm -rf "$target_root/public" "$target_root/private"

# Delete only rendered JSON. Cargo's compiled intermediate artifacts remain
# reusable, but a stale or extra blob cannot satisfy the exact metadata census.
# Sharing one directory makes this mandatory between the two passes, not just
# before the first: the copy below globs whatever *.json is present, so a blob
# left by the public pass could otherwise be counted as private output.
rm -rf "$census_target/doc"

log "--> rustdoc JSON public workspace census ($pin)"
# Raw cargo/rustdoc stderr names absolute scratch paths, source text, and
# signature tokens, none of which this gate may emit. Capture it inside the
# scratch root and report only a fixed label plus the exit status.
public_rc=0
(
  cd "$ROOT/packages"
  env \
    RUSTC_WRAPPER= \
    RUSTC_WORKSPACE_WRAPPER= \
    CARGO_BUILD_RUSTC_WRAPPER= \
    RUSTDOCFLAGS="-D warnings -Z unstable-options --output-format json" \
    cargo "+$pin" doc --locked --workspace --lib --no-deps \
      --target-dir "$census_target"
) >"$scratch/public-rustdoc.log" 2>&1 || public_rc=$?
[ "$public_rc" = 0 ] || {
  fail "api-surface public rustdoc JSON census failed (exit $public_rc); rerun 'make api-surface-pin' locally for compiler diagnostics" || true
  exit 1
}
find "$census_target/doc" -maxdepth 1 -type f -name '*.json' -exec cp -t "$public_dir" -- {} +

# Discard the public render before the private pass. Both passes emit into the
# same doc directory now, and the copy above has already taken what it needs.
rm -rf "$census_target/doc"

log "--> rustdoc JSON private + hidden workspace census ($pin)"
private_rc=0
(
  cd "$ROOT/packages"
  env \
    RUSTC_WRAPPER= \
    RUSTC_WORKSPACE_WRAPPER= \
    CARGO_BUILD_RUSTC_WRAPPER= \
    RUSTDOCFLAGS="-D warnings -Z unstable-options --output-format json --document-private-items --document-hidden-items" \
    cargo "+$pin" doc --locked --workspace --lib --no-deps \
      --target-dir "$census_target"
) >"$scratch/private-rustdoc.log" 2>&1 || private_rc=$?
[ "$private_rc" = 0 ] || {
  fail "api-surface private rustdoc JSON census failed (exit $private_rc); rerun 'make api-surface-pin' locally for compiler diagnostics" || true
  exit 1
}
find "$census_target/doc" -maxdepth 1 -type f -name '*.json' -exec cp -t "$private_dir" -- {} +

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
    fail "api-surface workspace metadata census drifted; regenerate with 'make api-surface-pin'"
    exit 1
  fi
}

mode=--check
[ "${D2B_API_SURFACE_UPDATE:-0}" = 1 ] && mode=--write
log "--> d2b-api-surface snapshot check"
(
cd "$ROOT/packages"
CARGO_TARGET_DIR="$checker_target" cargo run --quiet --locked \
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
)

ok "compiler-derived API and capability inventory"
