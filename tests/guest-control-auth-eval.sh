#!/usr/bin/env bash
# Guest-control auth token delivery eval invariants.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

scratch=$(nl_mktemp .guest-control-auth.XXXXXX)

NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { }" >/dev/null

if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { tokenFile = \"/nix/store\"; }" >/dev/null 2>&1; then
  fail "guest-control-auth-eval: /nix/store tokenFile unexpectedly passed"
fi

if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { tokenFile = \"/nix/store/not-a-token\"; }" >/dev/null 2>&1; then
  fail "guest-control-auth-eval: /nix/store/... tokenFile unexpectedly passed"
fi

if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { tokenFile = \"relative-token\"; }" >/dev/null 2>&1; then
  fail "guest-control-auth-eval: relative tokenFile unexpectedly passed"
fi

disabled_stderr=$scratch/disabled.stderr
if NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}" \
  nix eval --raw --impure --expr "import $ROOT/tests/guest-control-auth-eval.nix { guestControlEnable = false; }" >/dev/null 2>"$disabled_stderr"; then
  fail "guest-control-auth-eval: tokenFile without guest.control.enable unexpectedly passed"
fi
if ! grep -q "guest.control.auth.tokenFile is set" "$disabled_stderr"; then
  sed -n '1,40p' "$disabled_stderr" >&2 || true
  fail "guest-control-auth-eval: disabled tokenFile failure did not hit production assertion"
fi

ok "guest-control-auth-eval: token share and LoadCredential invariants hold"
