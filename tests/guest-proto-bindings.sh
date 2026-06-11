#!/usr/bin/env bash
# Validate generated guest-control protobuf message bindings.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

generated_dir="$ROOT/packages/nixling-ipc/src/generated"
generated_file="$generated_dir/guest_control.rs"

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  CARGO_BUILD_RUSTC_WRAPPER='' \
  RUSTC_WRAPPER='' \
  cargo run --manifest-path "$ROOT/packages/Cargo.toml" -p xtask -- gen-guest-proto >/dev/null

if ! git -C "$ROOT" diff --exit-code -- "$generated_dir" >/dev/null; then
  git -C "$ROOT" --no-pager diff -- "$generated_dir" | sed -n '1,160p' >&2
  fail "guest-proto-bindings: generated guest protobuf bindings drifted; run cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-guest-proto"
fi

if rg -n '\bunsafe\b|allow\(unsafe_code\)|expect\(unsafe_code\)|allow\(clippy::all\)|allow\(unknown_lints\)' "$generated_file"; then
  fail "guest-proto-bindings: generated bindings contain unsafe code or unsafe lint bypasses"
fi

if rg -n 'ttrpc|service GuestControl|GuestControl\\x12|Service|Client|Server|register_service|add_service|ServiceClient|ServiceServer' "$generated_file"; then
  fail "guest-proto-bindings: generated guest-control bindings must stay message-only"
fi

if rg -n 'ttrpc' "$ROOT/packages/nixling-ipc/Cargo.toml"; then
  fail "guest-proto-bindings: nixling-ipc must not depend on ttrpc for message-only bindings"
fi

ok "guest-proto-bindings: generated message bindings are deterministic and message-only"
