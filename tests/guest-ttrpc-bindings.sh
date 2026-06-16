#!/usr/bin/env bash
# Validate generated guestd-local ttRPC service bindings.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

generated_dir="$ROOT/packages/nixling-guestd/src/generated"
generated_file="$generated_dir/guest_control_ttrpc.rs"
guestd_manifest="$ROOT/packages/nixling-guestd/Cargo.toml"

command -v rg >/dev/null 2>&1 ||
  fail "guest-ttrpc-bindings: rg is required for static binding checks"

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  CARGO_BUILD_RUSTC_WRAPPER='' \
  RUSTC_WRAPPER='' \
  cargo run --locked --manifest-path "$ROOT/packages/Cargo.toml" -p xtask -- gen-guest-ttrpc >/dev/null

if ! git -C "$ROOT" diff --exit-code -- "$generated_dir" >/dev/null; then
  git -C "$ROOT" --no-pager diff -- "$generated_dir" | sed -n '1,160p' >&2
  fail "guest-ttrpc-bindings: generated guestd ttRPC bindings drifted; run cargo run --locked --manifest-path packages/Cargo.toml -p xtask -- gen-guest-ttrpc"
fi

if rg -n '\bunsafe\b|allow\(unsafe_code\)|expect\(unsafe_code\)|allow\(clippy::all\)|allow\(unknown_lints\)|clipto_camel_casepy' "$generated_file"; then
  fail "guest-ttrpc-bindings: generated service bindings contain unsafe code or broad lint bypasses"
fi

if [ -e "$ROOT/packages/nixling-guestd/build.rs" ]; then
  fail "guest-ttrpc-bindings: nixling-guestd must not generate ttRPC bindings during normal builds"
fi

if rg -n '^\[build-dependencies\]|ttrpc-codegen|ttrpc-compiler|prost-build|\bprotoc\b' "$guestd_manifest"; then
  fail "guest-ttrpc-bindings: nixling-guestd must keep ttRPC code generation in xtask only"
fi

if rg -n 'ttrpc' "$ROOT/packages/nixling-ipc/Cargo.toml"; then
  fail "guest-ttrpc-bindings: nixling-ipc must remain message-only and ttrpc-free"
fi

ok "guest-ttrpc-bindings: generated guestd service bindings are deterministic"
