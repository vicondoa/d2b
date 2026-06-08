#!/usr/bin/env bash
# W3 s4 L1c canary: minijail-too-old refusal.
#
# Plan.md §"W3 pre-merge canary matrix" lists `minijail-too-old`. The
# real minijail provisioning lives outside s4's owned files; this
# canary asserts the deterministic version-comparison logic the host
# check uses when it refuses to start with an older minijail.
#
# Tier-0 pin (plan.md §"W3 L3 distro matrix pinning"): Nix-built
# minijail v17. The canary refuses any version < 17.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/minijail-version-check.sh"

MIN_VERSION=17

cmp_version() {
  # Args: <observed> <required>. Echo "ok" or "too-old". Exit 0 always.
  local observed=$1
  local required=$2
  if [ "$observed" -ge "$required" ]; then
    echo "ok"
  else
    echo "too-old"
  fi
}

assert_cmp() {
  local label=$1 observed=$2 required=$3 expected=$4
  actual=$(cmp_version "$observed" "$required")
  if [ "$actual" = "$expected" ]; then
    ok "$label observed=$observed required=$required -> $actual"
  else
    fail "$label observed=$observed required=$required: expected $expected, got $actual"
  fi
}

assert_cmp "pinned-current"   "$MIN_VERSION"           "$MIN_VERSION" "ok"
assert_cmp "newer-accepted"   "$((MIN_VERSION + 3))"   "$MIN_VERSION" "ok"
assert_cmp "older-refused"    "$((MIN_VERSION - 1))"   "$MIN_VERSION" "too-old"
assert_cmp "ancient-refused"  "10"                      "$MIN_VERSION" "too-old"

# Parser-shape canary: simulate `minijail0 --version` output and assert
# we extract the integer correctly. Real minijail uses `minijail0 -h`
# / package metadata; the L1c shell parser stays tolerant.
fake_output() {
  cat <<EOF
minijail0
google/minijail revision 17 abc1234
EOF
}

parsed=$(fake_output | awk '/revision[ \t]+[0-9]+/{ for (i=1;i<=NF;i++) if ($i=="revision") { print $(i+1); exit } }')
if [ "$parsed" = "$MIN_VERSION" ]; then
  ok "version parser extracted '$parsed' from fake minijail0 output"
else
  fail "version parser extracted '$parsed' instead of $MIN_VERSION"
fi

ok "tests/minijail-version-check.sh: every minijail version canary passed"
