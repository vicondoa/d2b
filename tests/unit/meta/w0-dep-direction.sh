#!/usr/bin/env bash
# tests/unit/meta/w0-dep-direction.sh — ADR 0032 crate-granular dependency
# direction + lint-inheritance gate.
#
# The constellation contract crates must stay codec-/transport-/host-neutral:
#
#   * d2b-realm-core   — depends on NO other workspace crate and
#                                    NOT on prost (pure, codec-neutral model).
#   * d2b-realm-provider, -router, -transport (when present) —
#                                    may depend only on the contract crate(s)
#                                    listed below, and NOT on prost, a codec
#                                    crate, a transport impl crate, or any
#                                    host/broker/daemon crate.
#
# Every constellation crate must inherit workspace lints
# ([lints] workspace = true) so unsafe_code = "forbid" + clippy apply.
#
# Dependencies are resolved with `cargo metadata --no-deps` — the
# authoritative resolver: it returns each workspace member's dependencies by
# their REAL, resolved crate name (post-`package=` rename, post-`workspace =
# true` inheritance, including target-specific and any TOML-spelling form)
# without compiling or touching the network. The gate FAILS CLOSED if cargo
# metadata cannot be produced (the dependency-direction invariant cannot be
# verified without the resolver). cargo + jq are resolved from PATH, the
# rustup toolchain, ~/.cargo, or `nix run nixpkgs#<tool>`.
#
# Wired into `make test-policy` (tests/test-policy.sh) and tests/static.sh.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
PKGS="$ROOT/packages"

rc=0
note() { printf '  %s\n' "$*" >&2; }
violation() { printf 'FAIL: %s\n' "$*" >&2; rc=1; }

# ---------------------------------------------------------------------------
# Resolve cargo + jq (PATH, rustup toolchain, ~/.cargo, or `nix run`).
# ---------------------------------------------------------------------------
CARGO_BIN=""
if command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="$(command -v cargo)"
else
  for c in "$HOME"/.rustup/toolchains/*/bin/cargo "$HOME"/.cargo/bin/cargo; do
    [ -x "$c" ] && { CARGO_BIN="$c"; break; }
  done
fi
JQ_BIN=""
command -v jq >/dev/null 2>&1 && JQ_BIN="$(command -v jq)"
NIX_BIN=""
command -v nix >/dev/null 2>&1 && NIX_BIN="$(command -v nix)"

run_cargo() {
  if [ -n "$CARGO_BIN" ]; then "$CARGO_BIN" "$@"; return; fi
  [ -n "$NIX_BIN" ] || return 127
  "$NIX_BIN" run --no-write-lock-file nixpkgs#cargo -- "$@"
}
run_jq() {
  if [ -n "$JQ_BIN" ]; then "$JQ_BIN" "$@"; return; fi
  [ -n "$NIX_BIN" ] || return 127
  "$NIX_BIN" run --no-write-lock-file nixpkgs#jq -- "$@"
}

# ---------------------------------------------------------------------------
# Authoritative metadata.
# ---------------------------------------------------------------------------
META=""
if { [ -n "$CARGO_BIN" ] || [ -n "$NIX_BIN" ]; } \
   && { [ -n "$JQ_BIN" ] || [ -n "$NIX_BIN" ]; }; then
  META="$(run_cargo metadata --no-deps --format-version 1 \
    --manifest-path "$PKGS/Cargo.toml" 2>/dev/null || true)"
fi
if [ -z "$META" ] || ! printf '%s' "$META" | run_jq -e '.packages' >/dev/null 2>&1; then
  violation "cannot run 'cargo metadata' (need cargo + jq, or nix). The \
dependency-direction invariant cannot be verified; failing closed."
  exit "$rc"
fi

# The set of workspace member crate names (cargo metadata --no-deps lists
# only workspace members in .packages).
MEMBERS="$(printf '%s' "$META" | run_jq -r '.packages[].name' | sort -u)"
is_member() {
  printf '%s\n' "$MEMBERS" | grep -qxF "$1"
}

# External (non-member) crates a pure contract crate must never depend on.
# d2b-priv-broker lives in a SEPARATE workspace (excluded from
# packages/Cargo.toml), so it never appears in the member set — name it
# explicitly, along with any other d2b-* host/daemon crate caught by the
# glob in check_dep below.
is_external_forbidden() {
  case "$1" in
    prost | prost-types) return 0 ;;
    *) return 1 ;;
  esac
}

# Classify one resolved dependency name against a pure crate's allowlist.
# Forbidden iff it is NOT whitelisted and is any of: a workspace member
# (catches d2b, xtask, the sibling contract crates), any `d2b`/
# `d2b-*` crate (catches the separate-workspace d2b-priv-broker and
# every host/daemon crate, member or not), or an external forbidden crate
# (prost).
check_dep() {
  local crate="$1" allowed="$2" dep="$3"
  case "$allowed" in *" $dep "*) return 0 ;; esac
  case "$dep" in
    d2b | d2b-*)
      violation "$crate declares forbidden workspace/host dependency '$dep' (dependency-direction violation)"
      return 0
      ;;
  esac
  if is_member "$dep"; then
    violation "$crate declares forbidden workspace dependency '$dep' (dependency-direction violation)"
  elif is_external_forbidden "$dep"; then
    violation "$crate declares forbidden dependency '$dep' (dependency-direction violation)"
  fi
}

# Assert a manifest declares workspace lint inheritance (comments stripped so
# a commented-out `workspace = true` cannot satisfy the gate).
check_lints() {
  local manifest="$1" crate="$2"
  if ! awk '
      function strip(s) { sub(/#.*/, "", s); return s }
      /^[ \t]*\[lints\]/ { in_l = 1; next }
      /^[ \t]*\[/ { in_l = 0 }
      in_l { line = strip($0); if (line ~ /workspace[ \t]*=[ \t]*true/) found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$manifest"; then
    violation "$crate: missing [lints] workspace = true"
  fi
}

# Check a pure contract crate: every resolved non-dev dependency must be in
# the allowed list, else it is a dependency-direction violation.
check_pure_crate() {
  local crate="$1"; shift
  local allowed=" $* "
  local manifest="$PKGS/$crate/Cargo.toml"
  if ! is_member "$crate"; then
    note "skip $crate (not a workspace member yet)"
    return 0
  fi
  note "checking $crate (cargo metadata)"
  [ -f "$manifest" ] && check_lints "$manifest" "$crate"
  local dep
  # shellcheck disable=SC2016  # $c is a jq --arg variable, not a shell var.
  while IFS= read -r dep; do
    [ -n "$dep" ] && check_dep "$crate" "$allowed" "$dep"
  done < <(printf '%s' "$META" | run_jq -r --arg c "$crate" '
    .packages[] | select(.name == $c) | .dependencies[]
    | select(.kind != "dev") | .name')
}

# d2b-realm-core: depends on no workspace crate, no prost.
check_pure_crate d2b-realm-core
# d2b-realm-provider: only the core contract crate.
check_pure_crate d2b-realm-provider d2b-realm-core
# d2b-realm-router (s8): core + provider only, when it lands.
check_pure_crate d2b-realm-router \
  d2b-realm-core d2b-realm-provider
# d2b-realm-transport (s5): trait/mock home; core + provider only.
check_pure_crate d2b-realm-transport \
  d2b-realm-core d2b-realm-provider

# Prost stays confined to the protobuf codec crate (and a legitimate
# peer-session encoder, none yet); the checks above assert it never reaches a
# pure contract crate. The codec crate is intentionally NOT a pure crate.

if [ "$rc" -eq 0 ]; then
  printf 'w0-dep-direction OK\n' >&2
fi
exit "$rc"
