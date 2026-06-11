#!/usr/bin/env bash
# Auth-core non-goal guard: no listener/readiness/exec exposure in this slice.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if ! command -v rg >/dev/null 2>&1; then
  fail "guest-control-auth-nongoals: rg is required"
fi

if rg -n '\bttrpc\b|Listener::bind|Server::new|vsock://|tokio[_-]vsock|AF_VSOCK' \
  "$ROOT/packages/nixling-guestd/src" \
  "$ROOT/packages/nixling-guestd/Cargo.toml"; then
  fail "guest-control-auth-nongoals: guestd auth core must not wire a ttRPC/vsock listener yet"
fi

if rg -n '^\s*Exec(\b|\()' "$ROOT/packages/nixling/src/lib.rs"; then
  fail "guest-control-auth-nongoals: nixling exec CLI surface landed before exec runtime"
fi

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { }" >/dev/null

ok "guest-control-auth-nongoals: no listener, readiness, service activation, or exec CLI exposure"
