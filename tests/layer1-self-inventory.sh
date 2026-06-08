#!/usr/bin/env bash
# Fail closed when executable Layer-1 tests/*.sh scripts are not wired into static.sh.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

static="$ROOT/tests/static.sh"
if [ ! -f "$static" ]; then
  fail "missing tests/static.sh"
  exit 1
fi

is_known_non_layer1() {
  case "$1" in
    audio.sh|nixling-store.sh)
      # Documented Layer-2 integration tests in tests/README.md.
      return 0
      ;;
    audit-forwarding.sh|network-isolation.sh)
      # Documented optional Layer-2 live-host tests in tests/README.md.
      return 0
      ;;
    runner.sh|static-timing.sh)
      # Aggregating wrappers that invoke static.sh; not Layer-1 gate bodies.
      return 0
      ;;
    static-fast.sh)
      # W3a-3 PR-loop sibling tier (Tier 2 fast). Runs the
      # Layer-1 fast subset on its own; not invoked from static.sh.
      # See AGENTS.md "Build & validate" / docs/CHANGELOG W3a entry.
      return 0
      ;;
  esac
  return 1
}

missing=()

has_layer1_invocation() {
  local file="$1"
  local base="$2"

  awk -v base="$base" '
    function trim_comment(line) {
      sub(/^[[:space:]]+/, "", line)
      if (line ~ /^#/) {
        return ""
      }
      sub(/[[:space:]]+#.*/, "", line)
      return line
    }

    function re_escape(s) {
      gsub(/[][(){}.^$*+?|\\]/, "\\\\&", s)
      return s
    }

    {
      line = trim_comment($0)
      if (line == "") {
        next
      }

      quoted_base = re_escape(base)
      if (index(line, "bash \"$HERE/" base "\"") ||
          index(line, "bash \"$ROOT/tests/" base "\"") ||
          index(line, "bash $HERE/" base) ||
          index(line, "bash $ROOT/tests/" base) ||
          index(line, "nl_static_parallel_script \"tests/" base "\"") ||
          index(line, "nl_static_parallel_script_gate \"tests/" base "\"")) {
        found = 1
      }

      if (line ~ "(^|;|&&|\\|\\|)[[:space:]]*\"?\\$HERE/" quoted_base "\"?([[:space:];&|()]|$)" ||
          line ~ "(^|;|&&|\\|\\|)[[:space:]]*\"?\\$ROOT/tests/" quoted_base "\"?([[:space:];&|()]|$)" ||
          line ~ "(^|;|&&|\\|\\|)[[:space:]]*run_layer1_script[[:space:]]+\"?(tests/)?" quoted_base "\"?([[:space:];&|()]|$)" ||
          line ~ "(^|;|&&|\\|\\|)[[:space:]]*nl_static_parallel_script[[:space:]]+\"?(tests/)?" quoted_base "\"?([[:space:];&|()]|$)" ||
          line ~ "(^|;|&&|\\|\\|)[[:space:]]*nl_static_parallel_script_gate[[:space:]]+\"?(tests/)?" quoted_base "\"?([[:space:];&|()]|$)") {
        found = 1
      }
    }

    END { exit found ? 0 : 1 }
  ' "$file"
}

is_invoked_by_static() {
  local base="$1"
  local parent parent_base

  if has_layer1_invocation "$static" "$base"; then
    return 0
  fi

  while IFS= read -r parent; do
    parent_base=$(basename "$parent")
    if [ "$parent_base" = "$base" ]; then
      continue
    fi
    if has_layer1_invocation "$static" "$parent_base" \
      && has_layer1_invocation "$parent" "$base"; then
      return 0
    fi
  done < <(find "$ROOT/tests" -maxdepth 1 -type f -name '*.sh' -perm -u=x | LC_ALL=C sort)

  return 1
}

while IFS= read -r script; do
  base=$(basename "$script")
  case "$base" in
    lib.sh|static.sh|layer1-self-inventory.sh)
      continue
      ;;
  esac
  if is_known_non_layer1 "$base"; then
    continue
  fi
  if is_invoked_by_static "$base"; then
    ok "static.sh invokes $base"
  else
    missing+=( "tests/$base" )
  fi
done < <(find "$ROOT/tests" -maxdepth 1 -type f -name '*.sh' -perm -u=x | LC_ALL=C sort)

if [ "${#missing[@]}" -gt 0 ]; then
  fail "Layer-1 executable test(s) not invoked by tests/static.sh:" || true
  printf '  - %s\n' "${missing[@]}" >&2
  exit 1
fi

ok "Layer-1 test inventory is wired into tests/static.sh"
