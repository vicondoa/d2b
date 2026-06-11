#!/usr/bin/env bash
# Guest-control base-vsock allocation eval invariants.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

scratch=$(nl_mktemp .guest-control-vsock.XXXXXX)

eval_ok() {
  local scenario=$1
  NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
    nix eval --raw --impure --expr "import $ROOT/tests/guest-control-vsock-eval.nix { scenario = \"$scenario\"; }" >/dev/null
}

eval_fail() {
  local scenario=$1 expected=$2
  local stderr="$scratch/$scenario.stderr"
  : > "$stderr"
  if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
    nix eval --raw --impure --expr "import $ROOT/tests/guest-control-vsock-eval.nix { scenario = \"$scenario\"; }" >/dev/null 2>"$stderr"; then
    fail "guest-control-vsock-eval: $scenario unexpectedly passed"
  fi
  if ! grep -q "$expected" "$stderr"; then
    if [ -s "$stderr" ]; then
      sed -n '1,100p' "$stderr" >&2 || true
    else
      printf 'guest-control-vsock-eval: %s produced no stderr\n' "$scenario" >&2
    fi
    fail "guest-control-vsock-eval: $scenario failure did not contain '$expected'"
  fi
}

eval_ok base

eval_fail user-vsock-cid "read-only"
eval_fail user-vsock-socket "read-only"
eval_fail user-vsock-extra-split "must not set --vsock"
eval_fail user-vsock-extra-equals "must not set --vsock"
eval_fail long-socket "long for Linux AF_UNIX"

ok "guest-control-vsock-eval: base-vsock CID/socket parity and override guards hold"
