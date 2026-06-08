#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=tests/lib.sh
source "$ROOT/tests/lib.sh"

rendered=$(nl_smoke_bundle_privileges_json)
rust_matrix=$(
  perl -0ne '
    while (/row\(\s*"((?:\\"|[^"])*)"\s*,\s*"[^"]*"\s*,\s*"[^"]*"\s*,\s*&\[(.*?)\]\s*,/sg) {
      my ($op, $groups_raw) = ($1, $2);
      my @groups = $groups_raw =~ /"((?:\\"|[^"])*)"/g;
      print "$op\t", join(",", @groups), "\n";
    }
  ' "$ROOT/packages/nixling-core/src/privileges.rs" | LC_ALL=C sort
)

nix_matrix=$(
  jq -r '(.publicOperations[]),(.brokerOperations[]) | [.operation, (.allowedGroups | join(","))] | @tsv' "$rendered" | LC_ALL=C sort
)

rust_ops=$(printf '%s\n' "$rust_matrix" | cut -f1 | LC_ALL=C sort -u)
nix_ops=$(printf '%s\n' "$nix_matrix" | cut -f1 | LC_ALL=C sort -u)
rust_only=$(LC_ALL=C comm -23 <(printf '%s\n' "$rust_ops") <(printf '%s\n' "$nix_ops"))
nix_only=$(LC_ALL=C comm -13 <(printf '%s\n' "$rust_ops") <(printf '%s\n' "$nix_ops"))

if [ -n "$rust_only" ] || [ -n "$nix_only" ]; then
  printf 'privileges-json-rust-vs-nix-eval: FAIL — privileges matrix operation drift\n' >&2
  if [ -n "$rust_only" ]; then
    printf '  Rust-only ops:\n' >&2
    printf '%s\n' "$rust_only" | sed 's/^/    /' >&2
  fi
  if [ -n "$nix_only" ]; then
    printf '  Nix-only ops:\n' >&2
    printf '%s\n' "$nix_only" | sed 's/^/    /' >&2
  fi
  exit 1
fi

mismatches=$(
  LC_ALL=C join -t $'\t' <(printf '%s\n' "$rust_matrix") <(printf '%s\n' "$nix_matrix") \
    | awk -F '\t' '$2 != $3 { printf "%s\tRust=%s\tNix=%s\n", $1, $2, $3 }'
)

if [ -n "$mismatches" ]; then
  printf 'privileges-json-rust-vs-nix-eval: FAIL — allowedGroups drift between Rust and Nix privileges matrices\n' >&2
  printf '%s\n' "$mismatches" >&2
  exit 1
fi

printf 'privileges-json-rust-vs-nix-eval: PASS\n'
