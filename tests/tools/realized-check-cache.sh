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
usage: realized-check-cache.sh <import|export|plan|self-test> [check] [dir]

  import <check> <dir>  restore every path held in <dir> into the local store
  export <check> <dir>  publish <check>'s must-build inputs into <dir>
  plan   <check>        print the store paths export would publish
  self-test             round-trip an entry through a scratch store
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
# declare an output that a given build never realizes, and `nix copy` fails on
# a path it cannot read. Publishing is best-effort over whatever this run
# actually produced.
select_local_paths() {
  local path
  while read -r path; do
    [ -n "$path" ] || continue
    if nix path-info "$path" >/dev/null 2>&1; then
      printf '%s\n' "$path"
    fi
  done
}

# Print the store path of a derivation's default output, or every output when
# the default one cannot be identified.
#
# Nix names a derivation's default output after the derivation itself and
# appends `-<outputName>` to each of the others, so the default output is the
# one whose store-path name equals the derivation's name. Selecting it matters
# because a package built with separate debug info also produces a `debug`
# output, and nothing a realized check asserts can need one: measured on
# `video-binary-contract`, whose two must-build inputs it selects only `out`
# from, those two debug outputs were 145 of the entry's 175 MiB. Publishing
# only the default outputs takes the same entry to 30 MB.
#
# A derivation whose `outputs` omit `out` names its default output
# `<name>-<first>` instead, and then nothing matches. That case keeps every
# output rather than none: omitting a needed path costs a full rebuild, while
# carrying a few unneeded ones costs only bytes, so the ambiguous case errs
# towards completeness.
default_output_paths() {
  local drv=$1 name outs kept out out_name
  name=${drv##*/}
  name=${name#*-}
  name=${name%.drv}

  outs=$(nix-store --query --outputs "$drv")
  kept=""
  for out in $outs; do
    out_name=${out##*/}
    out_name=${out_name#*-}
    if [ "$out_name" = "$name" ]; then
      kept=$out
    fi
  done

  if [ -n "$kept" ]; then
    printf '%s\n' "$kept"
  else
    # shellcheck disable=SC2086 # deliberate split: one output path per word
    printf '%s\n' $outs
  fi
}

# Print the output paths of a check's direct build inputs that the upstream
# substituter does not already serve. Those are exactly the paths whose absence
# forces a local compile.
#
# This deliberately uses `nix-store --query`, whose plain-text output has been
# stable for years, rather than `nix derivation show` and jq. The JSON shape of
# the latter differs between Nix implementations - this tree evaluates under
# Lix while CI installs upstream Nix - and a shape mismatch there fails as an
# empty result rather than as an error, which published an empty entry once
# already. The references of a .drv are its input derivations and input
# sources, so selecting the `.drv` ones gives exactly the direct build inputs.
must_build_paths() {
  local check=$1 flake_ref native drv input out
  flake_ref=$(d2b_flake_ref "$ROOT")
  native=$(nix eval --raw --impure --expr builtins.currentSystem)

  drv=$(nix eval --raw --quiet --no-warn-dirty \
    "${flake_ref}#checks.${native}.${check}.drvPath")

  for input in $(nix-store --query --references "$drv" | grep '\.drv$'); do
    for out in $(default_output_paths "$input"); do
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

# The list of store paths an entry holds, written by `export` and consumed by
# `import`. It is named so it cannot collide with the binary-cache layout Nix
# writes into the same directory (`nix-cache-info`, `*.narinfo`, `nar/`), and
# Nix ignores files it does not recognise.
MANIFEST_NAME=d2b-cached-paths

cmd_import() {
  local check=$1 dir=$2 manifest paths path want missing
  : "$check"
  manifest="$dir/$MANIFEST_NAME"

  if [ ! -s "$manifest" ]; then
    log "  realized-check cache: no entry at $dir; the check will build normally"
    return 0
  fi

  paths=$(tr '\n' ' ' <"$manifest")

  # Copy the manifest's paths by name rather than asking the source store to
  # enumerate itself. `nix copy --all` reads as the obvious spelling and works
  # under Lix, which implements queryAllValidPaths for a local binary cache,
  # but upstream Nix - which is what CI installs - does not support that
  # operation on a binary-cache store and fails the whole copy. That divergence
  # cost a full run: the entry restored, the copy failed, and the check rebuilt
  # from scratch while the step still reported success.
  #
  # Best effort on purpose: a failed restore must cost time, never correctness.
  # --no-check-sigs because this store is written by this same workflow rather
  # than by a signing party, and its contents are only ever consumed by exact
  # store path.
  #
  # stderr is deliberately not discarded. The failure above was invisible
  # precisely because it was, and a restore that silently stops working has no
  # symptom other than the lane quietly never getting faster.
  # shellcheck disable=SC2086 # deliberate split: one store path per word
  nix copy --no-check-sigs --from "file://$dir" $paths || true

  # Report on what the store actually holds now rather than on the copy's exit
  # status. Whether the entry contributed is precisely the question of whether
  # these paths are valid, and deciding it here means any future way for the
  # restore to stop working - an unsupported operation, a corrupt entry, a
  # path the export never published - surfaces as this one warning instead of
  # as a lane that is mysteriously still slow.
  want=0
  missing=0
  while read -r path; do
    [ -n "$path" ] || continue
    want=$((want + 1))
    nix path-info "$path" >/dev/null 2>&1 || missing=$((missing + 1))
  done <"$manifest"

  if [ "$missing" -eq 0 ]; then
    log "  realized-check cache: restored $want path(s) from $dir"
  else
    log "  realized-check cache: $missing of $want path(s) missing after restore; building normally"
    printf '::warning::realized-check cache restored %s of %s path(s) for %s; this run rebuilds it\n' \
      "$((want - missing))" "$want" "$check"
  fi
  return 0
}

cmd_export() {
  local check=$1 dir=$2 paths

  paths=$(must_build_paths "$check" | select_local_paths)
  if [ -z "$paths" ]; then
    # Not fatal - publishing nothing costs a rebuild, never a wrong result, and
    # failing here would turn a passing check red. But it means the next run
    # pays the full build, so say so loudly enough to be noticed: this went
    # unnoticed once, as a silent empty entry, and the only symptom was the
    # lane never getting faster.
    log "  realized-check cache: nothing to publish for $check"
    printf '::warning::realized-check cache published nothing for %s; the next run rebuilds it\n' \
      "$check"
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
  printf '%s\n' "$paths" >"$dir/$MANIFEST_NAME"
  log "  realized-check cache: published $(printf '%s\n' "$paths" | wc -l) path(s) to $dir"
}

# A round trip through the exact copy shape `import` uses, plus a guard on the
# spelling that broke it. `nix copy --all` reads as the natural way to drain a
# restored entry and passes under Lix, so nothing local catches its
# reintroduction; upstream Nix rejects the operation on a binary-cache store
# and the only symptom is a lane that silently never gets faster.
cmd_self_test() {
  local work path cache store restored banned

  # Assembled rather than written out, so this line does not match itself.
  banned='nix copy'
  if grep -vE '^[[:space:]]*#' "$0" | grep -q -- "$banned .*--all"; then
    fail "realized-check cache: --all cannot enumerate a binary-cache store under upstream Nix"
  fi

  work=$(mktemp -d)
  # shellcheck disable=SC2064 # expand now: $work must not change before EXIT
  trap "rm -rf -- '$work'" EXIT

  printf 'realized-check cache self test\n' >"$work/probe"
  path=$(nix-store --add "$work/probe")
  cache="$work/cache"
  store="$work/store"
  mkdir -p "$store"

  nix copy --to "file://$cache?compression=zstd" "$path"
  printf '%s\n' "$path" >"$cache/$MANIFEST_NAME"
  [ -s "$cache/$MANIFEST_NAME" ] ||
    fail "realized-check cache: export wrote no manifest"

  restored=$(tr '\n' ' ' <"$cache/$MANIFEST_NAME")
  # shellcheck disable=SC2086 # deliberate split: one store path per word
  nix copy --no-check-sigs --from "file://$cache" --to "$store" $restored ||
    fail "realized-check cache: manifest round trip failed"

  [ -e "$store$path" ] ||
    fail "realized-check cache: $path absent from the restored store"

  log "realized-check cache self-test OK"
}

main() {
  [ $# -ge 1 ] || usage

  if [ "$1" = "self-test" ]; then
    cmd_self_test
    return 0
  fi

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
