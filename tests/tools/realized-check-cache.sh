#!/usr/bin/env bash
# tests/tools/realized-check-cache.sh - a targeted binary cache for the flake
# checks that must be BUILT rather than instantiated.
#
#   bash tests/tools/realized-check-cache.sh import <check> <dir>
#   bash tests/tools/realized-check-cache.sh export <check> <dir>
#   bash tests/tools/realized-check-cache.sh plan   <check>
#
# Why this exists. `video-binary-contract` is the only realized check, and it
# spends about sixteen minutes compiling a patched crosvm and a patched
# cloud-hypervisor so it can run each binary's `--help` and grep one flag out
# of the output. Measured on this tree, that compile has five direct build
# inputs, three of which (bash, gnugrep, stdenv) cache.nixos.org already
# serves. Only the two patched packages must ever be built, and their combined
# runtime closure exports to about 30 MB of zstd-compressed NAR.
#
# So the whole cost is recoverable by carrying those two outputs between runs.
# That is a far smaller object than a whole-store cache: the store cache the
# fixture-contract job uses is capped at 4G uncompressed, and the repository
# only gets ~10 GB of Actions cache in total (see NIX_CACHED_JOBS in
# tests/tools/layer1-jobs.py for why that budget forbids fanning store caches
# out across every nix job). A ~30 MB entry does not meaningfully compete for
# that budget, which is what makes caching this lane affordable at all.
#
# Why it cannot go wrong. Nix store paths are content-addressed by derivation
# hash, and `import` restores paths by exact name. Change a patch under pkgs/,
# bump flake.lock, or alter the derivation any other way, and the output path
# changes with it; the stale entry then simply fails to satisfy the new path
# and the build runs as before. A stale or wrong cache therefore costs a miss,
# never a wrong result - so the cache key only has to be good enough for a
# decent hit rate, and does not have to be sound.
#
# `import` is best-effort by design and never fails the job: not restoring a
# cache must degrade to the current behaviour (build it) rather than turn a
# green gate red.
#
# This is CI plumbing rather than a test case, so it lives under tests/tools/
# and needs no migration-ledger entry.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

# shellcheck source=tests/tools/flake-check-classes.sh
. "$HERE/flake-check-classes.sh"

usage() {
  cat >&2 <<'EOF'
usage: realized-check-cache.sh <import|export|plan> <check> [dir]

  import <check> <dir>  restore every path held in <dir> into the local store
  export <check> <dir>  publish <check>'s must-build inputs into <dir>
  plan   <check>        print the store paths export would publish
EOF
  return 2
}

# The substituter consulted to decide whether an input is worth carrying
# ourselves. A path this store already serves is cheaper to fetch than to
# cache, so it is excluded from the entry.
UPSTREAM_SUBSTITUTER=${D2B_UPSTREAM_SUBSTITUTER:-https://cache.nixos.org}

check_name_is_safe() {
  case "$1" in
    '' | *[!A-Za-z0-9._-]*) return 1 ;;
    *) return 0 ;;
  esac
}

# Keep only the paths that are valid in the local store. A derivation can
# declare outputs that a given build never realizes - the two patched packages
# each declare a `debug` output that is absent unless separateDebugInfo
# produced one - and `nix copy` fails on a path it cannot read. Publishing is
# best-effort over whatever this run actually produced.
select_local_paths() {
  local path
  while read -r path; do
    [ -n "$path" ] || continue
    if nix path-info "$path" >/dev/null 2>&1; then
      printf '%s\n' "$path"
    fi
  done
}

# Print the output paths of a check's direct build inputs that the upstream
# substituter does not already serve. Those are exactly the paths whose absence
# forces a local compile.
must_build_paths() {
  local check=$1 flake_ref native drv input out
  flake_ref=$(d2b_flake_ref "$ROOT")
  native=$(nix eval --raw --impure --expr builtins.currentSystem)

  drv=$(nix eval --raw --quiet --no-warn-dirty \
    "${flake_ref}#checks.${native}.${check}.drvPath")

  for input in $(nix derivation show "$drv" | jq -r '.[].inputDrvs | keys[]'); do
    for out in $(nix derivation show "$input" | jq -r '.[].outputs[].path'); do
      # A path upstream already has is not worth a slot in our entry.
      if ! nix path-info --store "$UPSTREAM_SUBSTITUTER" "$out" >/dev/null 2>&1; then
        printf '%s\n' "$out"
      fi
    done
  done
}

cmd_plan() {
  local check=$1
  must_build_paths "$check"
}

cmd_import() {
  local check=$1 dir=$2
  : "$check"

  if [ ! -d "$dir" ]; then
    log "  realized-check cache: no entry at $dir; the check will build normally"
    return 0
  fi

  # Best effort on purpose: a failed restore must cost time, never correctness.
  # --no-check-sigs because this store is written by this same workflow rather
  # than by a signing party, and its contents are only ever consumed by exact
  # store path.
  if nix copy --no-check-sigs --all --from "file://$dir" 2>/dev/null; then
    log "  realized-check cache: restored $dir"
  else
    log "  realized-check cache: restore from $dir failed; building normally"
  fi
  return 0
}

cmd_export() {
  local check=$1 dir=$2 paths

  paths=$(must_build_paths "$check" | select_local_paths)
  if [ -z "$paths" ]; then
    log "  realized-check cache: nothing to publish for $check"
    return 0
  fi

  # Publish into a fresh directory. A restore-keys hit can seed this with an
  # older run's paths, and appending to that would let the entry accumulate
  # every historical build without bound. Only the current paths are useful,
  # since anything else can no longer satisfy this derivation.
  rm -rf -- "$dir"
  mkdir -p "$dir"
  # shellcheck disable=SC2086
  nix copy --to "file://$dir?compression=zstd" $paths
  log "  realized-check cache: published $(printf '%s\n' "$paths" | wc -l) path(s) to $dir"
}

main() {
  [ $# -ge 2 ] || usage

  local mode=$1 check=$2
  shift 2

  check_name_is_safe "$check" ||
    fail "realized-check cache: check name '$check' has characters outside [A-Za-z0-9._-]"

  d2b_flake_check_is_realized "$check" ||
    fail "realized-check cache: '$check' is not a realized check"

  case "$mode" in
    plan)
      cmd_plan "$check"
      ;;
    import | export)
      [ $# -ge 1 ] || usage
      "cmd_$mode" "$check" "$1"
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
