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
eval_ok allow-root-only

eval_fail exec-no-control "guest.exec.enable requires"
eval_fail exec-disabled-users "guest.exec.allowRoot/users are set"
eval_fail exec-empty "no exec target is"
eval_fail duplicate-user "must not contain duplicate"
eval_fail root-user "must not include root"
eval_fail wildcard-user "must match"
eval_fail missing-user "declared as a normal or system user"
eval_fail internal-override "read-only"

ok "guest-exec-policy-eval: exec policy defaults, assertions, and dormant userd invariants hold"
