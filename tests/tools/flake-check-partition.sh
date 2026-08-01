#!/usr/bin/env bash
# tests/tools/flake-check-partition.sh - `make test-flake-partition`: split the
# native-system flake check names into the three CI dispatch classes and print
# them as `<key>=<json-array>` lines on stdout (all logs go to stderr).
#
# The output is shaped for `$GITHUB_OUTPUT` + `fromJSON()` directly:
#
#   bash tests/tools/flake-check-partition.sh >> "$GITHUB_OUTPUT"
#
# so one enumeration feeds every dispatch decision. Both discovery jobs consume
# this, which is what makes the exclusions safe: the names dropped from the
# eval matrix are exactly the names the Nix-unit lane is handed, and the names
# routed to the realized job are exactly the names test-flake.sh will build.
#
# This is plumbing for the sharded `make test-flake`, not a test case itself.
# It lives under tests/tools/ rather than tests/, so the migration ledger (which
# inventories tests/*.sh) does not need an entry for it.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# shellcheck source=tests/tools/flake-check-classes.sh
. "$HERE/flake-check-classes.sh"

cd "$ROOT"

# Renders a rejected element for a log line. Every byte outside the safe name
# charset becomes '?', and the result is length-bounded. The message exists to
# let a contributor locate the offending element, and its shape and position do
# that; the raw bytes must not reach a CI log, because this input is
# PR-controlled and a control sequence there could rewrite surrounding output.
# Truncating by parameter expansion rather than piping to head keeps a SIGPIPE
# out of the pipeline under `pipefail`.
render_rejected() {
  printf '%s' "${1:0:64}" | tr -c 'A-Za-z0-9._-' '?'
}

all_json=$(bash "$ROOT/tests/test-flake-list.sh")

# Read the names without a JSON parser. test-flake-list.sh emits a compact array
# of strings drawn from flake attrNames, so the shape is `["a","b"]`. Splitting
# on the separator is only safe once every element is known to be a quoted
# charset-safe token, so each token is validated as a whole: an element carrying
# a separator or a quote splits into pieces that fail that check and abort,
# rather than being silently dropped from the dispatch.
case "$all_json" in
  '['*']') ;;
  *)
    printf '%s\n' "flake-check-partition: enumeration is not a JSON array" >&2
    exit 1
    ;;
esac
inner=${all_json#[}
inner=${inner%]}

eval_names=()
realized_names=()
nix_unit_names=()
total=0
for token in $(printf '%s' "$inner" | tr ',' '\n'); do
  case "$token" in
    '"'?*'"') ;;
    *)
      printf '%s\n' "flake-check-partition: enumeration element is not a quoted name: $(render_rejected "$token")" >&2
      exit 1
      ;;
  esac
  name=${token#\"}
  name=${name%\"}
  case "$name" in
    *[!A-Za-z0-9._-]*)
      printf '%s\n' "flake-check-partition: a check name is outside [A-Za-z0-9._-]: $(render_rejected "$name")" >&2
      exit 1
      ;;
  esac
  total=$((total + 1))
  if d2b_flake_check_is_realized "$name"; then
    realized_names+=("$name")
  elif d2b_flake_check_is_nix_unit "$name"; then
    nix_unit_names+=("$name")
  else
    eval_names+=("$name")
  fi
done

# Fail closed on an empty or unreadable enumeration rather than emitting three
# empty matrices, which GitHub would report as a vacuously green flake gate.
[ "$total" -gt 0 ] || {
  printf '%s\n' "flake-check-partition: enumerated zero checks" >&2
  exit 1
}

# Every realized name must exist in the flake. Without this a typo in
# D2B_FLAKE_REALIZED_CHECKS would silently classify nothing, and the check it
# names would quietly move back into the instantiate-only matrix.
for name in $D2B_FLAKE_REALIZED_CHECKS; do
  found=0
  for present in ${realized_names[@]+"${realized_names[@]}"}; do
    [ "$present" = "$name" ] && found=1
  done
  [ "$found" = 1 ] || {
    printf '%s\n' "flake-check-partition: realized check '$name' is not a discovered flake check" >&2
    exit 1
  }
done

# The three classes are disjoint by construction above; assert they are also
# total, so a future class can never drop a check from every dispatch path.
partitioned=$(( ${#eval_names[@]} + ${#realized_names[@]} + ${#nix_unit_names[@]} ))
[ "$partitioned" = "$total" ] || {
  printf '%s\n' "flake-check-partition: partitioned $partitioned of $total checks" >&2
  exit 1
}

emit() {
  local key=$1 first=1 name
  shift
  printf '%s=[' "$key"
  for name in "$@"; do
    [ "$first" = 1 ] || printf ','
    first=0
    printf '"%s"' "$name"
  done
  printf ']\n'
}

printf '%s\n' "flake-check-partition: ${#eval_names[@]} eval, ${#realized_names[@]} realized, ${#nix_unit_names[@]} nix-unit (of $total)" >&2

emit evalchecks ${eval_names[@]+"${eval_names[@]}"}
emit realizedchecks ${realized_names[@]+"${realized_names[@]}"}
emit nixunitchecks ${nix_unit_names[@]+"${nix_unit_names[@]}"}
