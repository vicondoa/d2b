#!/usr/bin/env bash
# Fast freshness check for the compiler-derived API census inputs.
set -euo pipefail
export LC_ALL=C

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
PIN="$ROOT/tests/golden/api-surface/input-fingerprint.txt"
MODE=${1:---check}

case "$MODE" in
  --check|--write) ;;
  *)
    printf '%s\n' "usage: $0 [--check|--write]" >&2
    exit 2
    ;;
esac

inputs=(
  packages/Cargo.toml
  packages/Cargo.lock
  packages/rust-toolchain.toml
  tests/golden/api-surface/roots.json
  tests/tools/api-surface-json.sh
  tests/tools/api-surface-input-fingerprint.sh
  tests/tools/gen-api-surface-metadata.sh
)

for crate_dir in "$ROOT"/packages/*; do
  [ -d "$crate_dir" ] || continue
  case "${crate_dir##*/}" in
    d2b-priv-broker|d2b-guest-shell-runner|target)
      continue
      ;;
  esac
  [ -f "$crate_dir/Cargo.toml" ] || continue
  inputs+=("${crate_dir#"$ROOT/"}"/Cargo.toml)
  [ ! -f "$crate_dir/build.rs" ] || inputs+=("${crate_dir#"$ROOT/"}"/build.rs)
  if [ -d "$crate_dir/src" ]; then
    while IFS= read -r source; do
      inputs+=("${source#"$ROOT/"}")
    done < <(find "$crate_dir/src" -type f -name '*.rs' | sort)
  fi
done

mapfile -t inputs < <(printf '%s\n' "${inputs[@]}" | sort -u)
for path in "${inputs[@]}"; do
  [ -f "$ROOT/$path" ] || {
    printf '%s\n' "api-surface input is missing: $path" >&2
    exit 1
  }
done

digest=$(
  {
    printf 'd2b-api-surface-inputs-v1\0'
    for path in "${inputs[@]}"; do
      printf '%s\0' "$path"
      sha256sum "$ROOT/$path" | cut -d' ' -f1
      printf '\0'
    done
  } | sha256sum | cut -d' ' -f1
)
printf -v expected 'version=1\nsha256=%s\n' "$digest"

if [ "$MODE" = "--write" ]; then
  mkdir -p "$(dirname "$PIN")"
  temporary=$(mktemp "$PIN.tmp.XXXXXX")
  trap 'rm -f -- "$temporary"' EXIT
  printf '%s' "$expected" > "$temporary"
  chmod 0644 "$temporary"
  mv -f "$temporary" "$PIN"
  trap - EXIT
  printf '%s\n' "api-surface input fingerprint updated"
  exit 0
fi

if [ ! -f "$PIN" ] || ! printf '%s' "$expected" | cmp -s - "$PIN"; then
  printf '%s\n' \
    "api-surface inputs changed since the compiler-derived snapshots were generated; run 'make api-surface-pin'" \
    >&2
  exit 1
fi

printf '%s\n' "api-surface input fingerprint is current"
