#!/usr/bin/env bash
# Emit the store paths whose source builds dominate fixture-contract CI.
#
# This is deliberately a selective cache rather than the whole fixture output
# closure: the latter contains full NixOS systems and is far beyond the GitHub
# cache budget. All selected paths are still imported through `nix-store`, so
# signatures/hashes and transitive references remain Nix's authority.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

cd "$ROOT"

system=$(nix eval --raw --impure --expr builtins.currentSystem)
mapfile -t fixture_paths < <(
  nix eval --raw --impure ".#checks.$system" --apply '
    checks:
      builtins.concatStringsSep "\n" [
        (toString checks.fixture-smoke)
        (toString checks.fixture-smoke-full)
      ]
  '
)

candidate_paths=()
for fixture in "${fixture_paths[@]}"; do
  [ -e "$fixture/processes.json" ] || {
    echo "fixture cache source is missing processes.json: $fixture" >&2
    exit 1
  }
  while IFS= read -r path; do
    [ -n "$path" ] && candidate_paths+=("$path")
  done < <(
    grep -oE '/nix/store/[a-z0-9]{32}-[^" ,/]+' "$fixture/processes.json" \
      | LC_ALL=C sort -u
  )
  for binary in d2b-guestd d2b-exec-runner; do
    while IFS= read -r path; do
      [ -n "$path" ] && candidate_paths+=("$path")
    done < <(
      nix-store --query --requisites "$fixture" \
        | grep -E -- "-${binary}-static-static-" \
        | LC_ALL=C sort -u \
        || true
    )
  done
done

mapfile -t selected_paths < <(
  printf '%s\n' "${candidate_paths[@]}" \
    | grep -E -- '-(cloud-hypervisor|crosvm($|-)|d2b-wayland-proxy|d2b-guestd-static|d2b-exec-runner-static)' \
    | LC_ALL=C sort -u
)

[ "${#selected_paths[@]}" -gt 0 ] || {
  echo "fixture cache selector found no expensive derivations" >&2
  exit 1
}
for family in cloud-hypervisor crosvm d2b-wayland-proxy d2b-guestd-static d2b-exec-runner-static; do
  printf '%s\n' "${selected_paths[@]}" | grep -F -- "-$family" >/dev/null || {
    echo "fixture cache selector is missing required family: $family" >&2
    exit 1
  }
done
printf '%s\n' "${selected_paths[@]}"
