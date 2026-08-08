#!/usr/bin/env bash
# Fast freshness check for the compiler-derived API census inputs.
set -euo pipefail
export LC_ALL=C

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
PIN="$ROOT/tests/golden/api-surface/input-fingerprint.txt"
MODE=${1:---check}
enumeration_file=
temporary=

cleanup() {
  [ -z "$enumeration_file" ] || rm -f -- "$enumeration_file"
  [ -z "$temporary" ] || rm -f -- "$temporary"
}
trap cleanup EXIT

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

enumeration_file=$(mktemp "${TMPDIR:-/tmp}/d2b-api-surface-inputs.XXXXXX")
package_root="$ROOT/packages"
if [ ! -d "$package_root" ] || [ -L "$package_root" ]; then
  printf '%s\n' "api-surface package root is missing or has an unexpected type" >&2
  exit 1
fi
if ! find "$package_root" -mindepth 1 -maxdepth 1 -print0 \
    | sort -z >"$enumeration_file"; then
  printf '%s\n' "api-surface package enumeration failed" >&2
  exit 1
fi
mapfile -d '' -t package_entries <"$enumeration_file"

crate_count=0
for package_entry in "${package_entries[@]}"; do
  entry_name=${package_entry##*/}
  case "$entry_name" in
    .cargo|.config|d2b-priv-broker|d2b-guest-shell-runner|target)
      if [ ! -d "$package_entry" ] || [ -L "$package_entry" ]; then
        printf '%s\n' \
          "api-surface package entry has an unexpected type: packages/$entry_name" >&2
        exit 1
      fi
      continue
      ;;
    Cargo.guest.lock|Cargo.lock|Cargo.toml|deny.toml|rust-toolchain.toml)
      if [ ! -f "$package_entry" ] || [ -L "$package_entry" ]; then
        printf '%s\n' \
          "api-surface package entry has an unexpected type: packages/$entry_name" >&2
        exit 1
      fi
      continue
      ;;
  esac

  if [ ! -d "$package_entry" ] || [ -L "$package_entry" ]; then
    printf '%s\n' \
      "api-surface package entry has an unexpected type: packages/$entry_name" >&2
    exit 1
  fi
  crate_dir=$package_entry
  if [ ! -f "$crate_dir/Cargo.toml" ] || [ -L "$crate_dir/Cargo.toml" ]; then
    printf '%s\n' \
      "api-surface crate manifest is missing or has an unexpected type: packages/$entry_name/Cargo.toml" >&2
    exit 1
  fi
  crate_count=$((crate_count + 1))
  inputs+=("${crate_dir#"$ROOT/"}"/Cargo.toml)
  if [ -e "$crate_dir/build.rs" ] || [ -L "$crate_dir/build.rs" ]; then
    if [ ! -f "$crate_dir/build.rs" ] || [ -L "$crate_dir/build.rs" ]; then
      printf '%s\n' \
        "api-surface input has an unexpected type: packages/$entry_name/build.rs" >&2
      exit 1
    fi
    inputs+=("${crate_dir#"$ROOT/"}"/build.rs)
  fi
  if [ -e "$crate_dir/src" ] || [ -L "$crate_dir/src" ]; then
    if [ ! -d "$crate_dir/src" ] || [ -L "$crate_dir/src" ]; then
      printf '%s\n' \
        "api-surface source root has an unexpected type: packages/$entry_name/src" >&2
      exit 1
    fi
    if ! find "$crate_dir/src" -mindepth 1 -print0 \
        | sort -z >"$enumeration_file"; then
      printf '%s\n' \
        "api-surface source enumeration failed: packages/$entry_name/src" >&2
      exit 1
    fi
    while IFS= read -r -d '' source; do
      if [ -L "$source" ] || { [ ! -f "$source" ] && [ ! -d "$source" ]; }; then
        printf '%s\n' \
          "api-surface input has an unexpected type: ${source#"$ROOT/"}" >&2
        exit 1
      fi
      if [ -f "$source" ] && [[ "$source" == *.rs ]]; then
        inputs+=("${source#"$ROOT/"}")
      fi
    done <"$enumeration_file"
  fi
done

if [ "$crate_count" -eq 0 ]; then
  printf '%s\n' "api-surface package enumeration selected no workspace crates" >&2
  exit 1
fi

if ! printf '%s\0' "${inputs[@]}" | sort -zu >"$enumeration_file"; then
  printf '%s\n' "api-surface input ordering failed" >&2
  exit 1
fi
inputs=()
mapfile -d '' -t inputs <"$enumeration_file"
for path in "${inputs[@]}"; do
  if [ ! -e "$ROOT/$path" ] && [ ! -L "$ROOT/$path" ]; then
    printf '%s\n' "api-surface input is missing: $path" >&2
    exit 1
  fi
  if [ ! -f "$ROOT/$path" ] || [ -L "$ROOT/$path" ]; then
    printf '%s\n' "api-surface input has an unexpected type: $path" >&2
    exit 1
  fi
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
  printf '%s' "$expected" > "$temporary"
  chmod 0644 "$temporary"
  mv -f "$temporary" "$PIN"
  temporary=
  printf '%s\n' "api-surface input fingerprint updated"
  exit 0
fi

if [ ! -f "$PIN" ] || [ -L "$PIN" ] \
    || ! printf '%s' "$expected" | cmp -s - "$PIN"; then
  printf '%s\n' \
    "api-surface inputs changed since the compiler-derived snapshots were generated; run 'make api-surface-pin'" \
    >&2
  exit 1
fi

printf '%s\n' "api-surface input fingerprint is current"
