#!/usr/bin/env bash
# Guest exec policy eval invariants.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

scratch=$(nl_mktemp .guest-exec-policy.XXXXXX)

eval_ok() {
  local scenario=$1
  NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
    nix eval --raw --impure --expr "import $ROOT/tests/guest-exec-policy-eval.nix { scenario = \"$scenario\"; }" >/dev/null
}

eval_fail() {
  local scenario=$1 expected=$2
  local stderr="$scratch/$scenario.stderr"
  if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
    nix eval --raw --impure --expr "import $ROOT/tests/guest-exec-policy-eval.nix { scenario = \"$scenario\"; }" >/dev/null 2>"$stderr"; then
    fail "guest-exec-policy-eval: $scenario unexpectedly passed"
  fi
  if ! grep -q "$expected" "$stderr"; then
    sed -n '1,80p' "$stderr" >&2 || true
    fail "guest-exec-policy-eval: $scenario failure did not contain '$expected'"
  fi
}

eval_ok enabled
eval_ok default
eval_ok control-no-exec
eval_ok detached-ceiling
eval_ok interactive-ceiling

eval_fail exec-no-control "guest.exec.enable requires"
eval_fail exec-no-user "no workload user"
eval_fail root-user "must not be root"
eval_fail invalid-user "must match"
eval_fail missing-user "declared as a normal"

ok "guest-exec-policy-eval: workload-user exec policy defaults and assertions hold"
