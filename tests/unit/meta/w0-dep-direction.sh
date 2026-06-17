#!/usr/bin/env bash
# tests/unit/meta/w0-dep-direction.sh — ADR 0032 crate-granular dependency
# direction + lint-inheritance gate.
#
# The constellation contract crates must stay codec-/transport-/host-neutral:
#
#   * nixling-constellation-core   — depends on NO other workspace crate and
#                                    NOT on prost (pure, codec-neutral model).
#   * nixling-constellation-provider, -router (when present) — may depend only
#                                    on the contract crate(s) listed below, and
#                                    NOT on prost, a codec crate, a transport
#                                    impl crate, or any host/broker/daemon crate.
#
# Every constellation crate must inherit workspace lints ([lints] workspace =
# true) so unsafe_code = "forbid" + clippy apply.
#
# The gate parses each crate's declared [dependencies] directly (it does not
# need cargo on PATH): a forbidden edge cannot exist without being declared.
# It is wired into `make test-policy` via tests/test-policy.sh.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
PKGS="$ROOT/packages"

rc=0
note() { printf '  %s\n' "$*" >&2; }
violation() { printf 'FAIL: %s\n' "$*" >&2; rc=1; }

# Print the declared dependency names of a crate manifest. Handles
# `name = ...`, `name.workspace = true`, `[dependencies.name]` tables,
# target-specific `[target.'cfg(...)'.dependencies(.name)]` sections, inline
# `package = "real"` / `package = 'real'` renames (both quote styles), and
# trailing comments on section headers. Ignores dev-/build-dependencies.
crate_deps() {
  local manifest="$1"
  awk '
    {
      line = $0
      sub(/#.*/, "", line)              # strip trailing comment
      sub(/[ \t]+$/, "", line)          # strip trailing whitespace
      gsub(/^[ \t]+/, "", line)         # strip leading whitespace
      if (line == "") next

      if (line ~ /^\[/) {               # a section header
        in_deps = (line == "[dependencies]") \
                  || (line ~ /^\[target\./ && line ~ /\.dependencies\]$/)
        in_table = 0
        # dependency TABLE header: [dependencies.NAME] or
        # [target.<cfg>.dependencies.NAME]
        if (line ~ /^\[dependencies\.[^]]+\]$/ \
            || (line ~ /^\[target\./ && line ~ /\.dependencies\.[^]]+\]$/)) {
          name = line
          sub(/\]$/, "", name)
          if (name ~ /^\[dependencies\./) sub(/^\[dependencies\./, "", name)
          else sub(/^.*\.dependencies\./, "", name)
          gsub(/["'"'"' ]/, "", name)
          if (name != "") { print name; in_table = 1 }
        }
        next
      }

      # `package = "real"` / `package = '"'"'real'"'"'` rename target.
      if (line ~ /(^|[,{ \t])package[ \t]*=/) {
        pkg = line
        sub(/.*package[ \t]*=[ \t]*["'"'"']/, "", pkg)
        sub(/["'"'"'].*$/, "", pkg)
        if (pkg != "") print pkg
      }

      if (in_deps) {                    # a key in a flat dependency section
        key = line
        sub(/[ \t]*[=.].*$/, "", key)
        gsub(/["'"'"']/, "", key)
        if (key != "") print key
      }
    }
  ' "$manifest"
}

# Assert a manifest declares workspace lint inheritance (comments stripped
# so a commented-out `workspace = true` cannot satisfy the gate).
check_lints() {
  local manifest="$1" crate="$2"
  if ! awk '
      function strip(s) { sub(/#.*/, "", s); return s }
      /^\[/ { in_l = ($0 ~ /^\[lints\]/); next }
      in_l { line = strip($0); if (line ~ /workspace[ \t]*=[ \t]*true/) found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$manifest"; then
    violation "$crate: missing [lints] workspace = true"
  fi
}

# Host/broker/daemon + codec/transport-impl crates a pure contract crate must
# never depend on. (The contract crates themselves are added to the allowed
# list per-crate below.)
is_forbidden_edge() {
  local dep="$1"
  case "$dep" in
    prost | prost-types) return 0 ;;
    nixling-constellation-codec-* ) return 0 ;;
    nixling-constellation-transport ) return 0 ;;
    nixling-core | nixling-ipc | nixling-host | nixling-host-activation-helper \
    | nixling-priv-broker | nixlingd | nixling-guestd | nixling-userd \
    | nixling-exec-runner | nixling-wayland-filter ) return 0 ;;
    *) return 1 ;;
  esac
}

# Check a pure contract crate: every declared dependency that is a workspace
# crate or prost must be in the allowed list; forbidden edges fail closed.
check_pure_crate() {
  local crate="$1"; shift
  local allowed=" $* "
  local manifest="$PKGS/$crate/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    note "skip $crate (not present yet)"
    return 0
  fi
  note "checking $crate"
  check_lints "$manifest" "$crate"
  local dep
  while IFS= read -r dep; do
    [ -n "$dep" ] || continue
    # Allowed contract deps are explicitly whitelisted.
    case "$allowed" in *" $dep "*) continue ;; esac
    if is_forbidden_edge "$dep"; then
      violation "$crate declares forbidden dependency '$dep' (dependency-direction violation)"
    fi
    # A pure crate may not depend on ANY other nixling-* crate not whitelisted.
    case "$dep" in
      nixling-*)
        violation "$crate declares un-whitelisted workspace dependency '$dep'"
        ;;
    esac
  done < <(crate_deps "$manifest")
}

# nixling-constellation-core: depends on no workspace crate, no prost.
check_pure_crate nixling-constellation-core
# nixling-constellation-provider: only the core contract crate.
check_pure_crate nixling-constellation-provider nixling-constellation-core
# nixling-constellation-router (s8): core + provider only, when it lands.
check_pure_crate nixling-constellation-router \
  nixling-constellation-core nixling-constellation-provider
# nixling-constellation-transport (s5): trait/mock home; core + provider only.
check_pure_crate nixling-constellation-transport \
  nixling-constellation-core nixling-constellation-provider

# Prost allowlist: prost may appear ONLY in the protobuf codec crate (and a
# legitimate peer-session encoder, none yet). Assert it is absent from the
# contract crates above (already covered) — and that no NEW pure crate sneaks
# a prost edge in. The protobuf codec crate is intentionally NOT checked here.

if [ "$rc" -eq 0 ]; then
  printf 'w0-dep-direction OK\n' >&2
fi
exit "$rc"
