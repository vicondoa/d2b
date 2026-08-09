#!/usr/bin/env bash
# Fast freshness check for the compiler-derived API census inputs.
set -euo pipefail
export LC_ALL=C

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
PIN="$ROOT/tests/golden/api-surface/input-fingerprint.txt"
MODE=${1:---check}
enumeration_file=
workspace_manifest_enumeration=
temporary=

cleanup() {
  [ -z "$enumeration_file" ] || rm -f -- "$enumeration_file"
  [ -z "$workspace_manifest_enumeration" ] \
    || rm -f -- "$workspace_manifest_enumeration"
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
  packages/.cargo/config.toml
  packages/rust-toolchain.toml
  packages/d2b-api-surface/rust-toolchain.toml
  tests/golden/api-surface/roots.json
  tests/tools/api-surface-json.sh
  tests/tools/api-surface-input-fingerprint.sh
  tests/tools/gen-api-surface-metadata.sh
)

independent_workspace_roots=(
  packages/d2b-bus/tests/ui/public-api-mutations
  packages/d2b-controller-toolkit/tests/ui/external-seals
  packages/d2b-core/fuzz
  packages/d2b-guest-shell-runner
  packages/d2b-priv-broker
  packages/d2b-resource-api/tests/ui/external-seals
  packages/d2b-wlproxy-spike
)

is_independent_workspace_root() {
  local candidate="$1"
  local allowed
  for allowed in "${independent_workspace_roots[@]}"; do
    [ "$candidate" = "$allowed" ] && return 0
  done
  return 1
}

package_root="$ROOT/packages"
if [ ! -d "$package_root" ] || [ -L "$package_root" ]; then
  printf '%s\n' "api-surface package root is missing or has an unexpected type" >&2
  exit 1
fi
workspace_manifest="$package_root/Cargo.toml"
if [ ! -f "$workspace_manifest" ] || [ -L "$workspace_manifest" ]; then
  printf '%s\n' "api-surface workspace manifest is missing or has an unexpected type" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  printf '%s\n' "api-surface workspace membership requires cargo metadata" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  printf '%s\n' "api-surface workspace membership requires jq" >&2
  exit 1
fi

metadata=
if ! metadata=$(cargo metadata --locked --offline --no-deps --format-version 1 \
    --manifest-path "$workspace_manifest"); then
  printf '%s\n' "api-surface workspace metadata enumeration failed" >&2
  exit 1
fi
if ! printf '%s' "$metadata" | jq -e '
      type == "object"
      and (has("workspace_root") and (.workspace_root | type == "string"))
      and (has("workspace_members")
        and (.workspace_members | type == "array" and length > 0
          and all(.[]; type == "string")))
      and (has("packages")
        and (.packages | type == "array" and length > 0
          and all(.[]; type == "object"
            and has("id") and (.id | type == "string")
            and has("name") and (.name | type == "string")
            and has("manifest_path") and (.manifest_path | type == "string"))))
      and ([. as $metadata
        | $metadata.packages[]
        | select(.id as $id | $metadata.workspace_members | index($id))]
        | length > 0)
    ' >/dev/null; then
  printf '%s\n' \
    "api-surface workspace metadata did not describe any workspace crates" >&2
  exit 1
fi

workspace_metadata_root=$(printf '%s' "$metadata" | jq -r '.workspace_root')
workspace_root_real=$(readlink -f -- "$workspace_metadata_root") || {
  printf '%s\n' "api-surface workspace metadata root could not be resolved" >&2
  exit 1
}
package_root_real=$(readlink -f -- "$package_root") || {
  printf '%s\n' "api-surface package root could not be resolved" >&2
  exit 1
}
if [ "$workspace_root_real" != "$package_root_real" ]; then
  printf '%s\n' \
    "api-surface workspace metadata root does not match packages/" >&2
  exit 1
fi

# Every independent workspace under packages/ is a deliberate, closed
# exception. Checking all nested manifests prevents an unclassified workspace
# from disappearing beneath an ordinary workspace member.
workspace_manifest_enumeration=$(
  mktemp "${TMPDIR:-/tmp}/d2b-api-surface-workspaces.XXXXXX"
)
if ! find -P "$package_root" -type f -name Cargo.toml -print0 \
    | sort -z >"$workspace_manifest_enumeration"; then
  printf '%s\n' "api-surface package enumeration failed" >&2
  exit 1
fi
while IFS= read -r -d '' workspace_manifest; do
  if ! grep -Eq '^[[:space:]]*\[workspace\][[:space:]]*$' \
      "$workspace_manifest"; then
    continue
  fi
  workspace_root=${workspace_manifest#"$ROOT/"}
  workspace_root=${workspace_root%/Cargo.toml}
  [ "$workspace_root" = "packages" ] && continue
  if ! is_independent_workspace_root "$workspace_root"; then
    printf '%s\n' \
      "api-surface unknown independent workspace root: $workspace_root" >&2
    exit 1
  fi
done <"$workspace_manifest_enumeration"
rm -f -- "$workspace_manifest_enumeration"
workspace_manifest_enumeration=

declare -A workspace_member_roots=()
member_rows=$(
  printf '%s' "$metadata" | jq -r '
    . as $metadata
    | $metadata.packages[]
    | select(.id as $id | $metadata.workspace_members | index($id))
    | [.name, .manifest_path]
    | @tsv
  '
) || {
  printf '%s\n' "api-surface workspace package enumeration failed" >&2
  exit 1
}

crate_count=0
while IFS=$'\t' read -r package_name manifest_path; do
  [ -n "$package_name" ] || continue
  [ -n "$manifest_path" ] || {
    printf '%s\n' \
      "api-surface workspace package has no manifest path: $package_name" >&2
    exit 1
  }
  if [ ! -f "$manifest_path" ] || [ -L "$manifest_path" ]; then
    printf '%s\n' \
      "api-surface crate manifest is missing or has an unexpected type: $manifest_path" >&2
    exit 1
  fi
  manifest_real=$(readlink -f -- "$manifest_path") || {
    printf '%s\n' \
      "api-surface crate manifest could not be resolved: $manifest_path" >&2
    exit 1
  }
  crate_dir=$(dirname "$manifest_path")
  crate_dir_real=$(readlink -f -- "$crate_dir") || {
    printf '%s\n' \
      "api-surface crate directory could not be resolved: $manifest_path" >&2
    exit 1
  }
  case "$crate_dir_real" in
    "$package_root_real"/*) ;;
    *)
      printf '%s\n' \
        "api-surface workspace crate is outside packages/: $manifest_path" >&2
      exit 1
      ;;
  esac
  workspace_member_roots["$crate_dir_real"]=1
  crate_count=$((crate_count + 1))
  inputs+=("${manifest_real#"$ROOT/"}")
  if [ -e "$crate_dir/build.rs" ] || [ -L "$crate_dir/build.rs" ]; then
    if [ ! -f "$crate_dir/build.rs" ] || [ -L "$crate_dir/build.rs" ]; then
      printf '%s\n' \
        "api-surface input has an unexpected type: ${crate_dir#"$ROOT/"}/build.rs" >&2
      exit 1
    fi
    inputs+=("${crate_dir#"$ROOT/"}/build.rs")
  fi
  if [ -e "$crate_dir/src" ] || [ -L "$crate_dir/src" ]; then
    if [ ! -d "$crate_dir/src" ] || [ -L "$crate_dir/src" ]; then
      printf '%s\n' \
        "api-surface source root has an unexpected type: ${crate_dir#"$ROOT/"}/src" >&2
      exit 1
    fi
    enumeration_file=$(mktemp "${TMPDIR:-/tmp}/d2b-api-surface-inputs.XXXXXX")
    if ! find "$crate_dir/src" -mindepth 1 -print0 \
        | sort -z >"$enumeration_file"; then
      rm -f -- "$enumeration_file"
      printf '%s\n' \
        "api-surface package enumeration failed while enumerating ${crate_dir#"$ROOT/"}/src" >&2
      exit 1
    fi
    while IFS= read -r -d '' source; do
      if [ -L "$source" ] || { [ ! -f "$source" ] && [ ! -d "$source" ]; }; then
        rm -f -- "$enumeration_file"
        printf '%s\n' \
          "api-surface input has an unexpected type: ${source#"$ROOT/"}" >&2
        exit 1
      fi
      if [ -f "$source" ] && [[ "$source" == *.rs ]]; then
        inputs+=("${source#"$ROOT/"}")
      fi
    done <"$enumeration_file"
    rm -f -- "$enumeration_file"
  fi
done <<<"$member_rows"

if [ "$crate_count" -eq 0 ]; then
  printf '%s\n' "api-surface package enumeration selected no workspace crates" >&2
  exit 1
fi

enumeration_file=$(mktemp "${TMPDIR:-/tmp}/d2b-api-surface-inputs.XXXXXX")
if ! find "$package_root" -mindepth 1 -maxdepth 1 -print0 \
    | sort -z >"$enumeration_file"; then
  printf '%s\n' "api-surface package enumeration failed" >&2
  exit 1
fi
while IFS= read -r -d '' package_entry; do
  entry_name=${package_entry##*/}
  entry_real=$(readlink -f -- "$package_entry") || {
    printf '%s\n' \
      "api-surface package entry could not be resolved: packages/$entry_name" >&2
    exit 1
  }
  member_entry=0
  for member_root in "${!workspace_member_roots[@]}"; do
    case "$entry_real" in
      "$member_root"|"$member_root"/*)
        member_entry=1
        break
        ;;
    esac
  done
  [ "$member_entry" -eq 1 ] && continue

  case "$entry_name" in
    Cargo.guest.lock|Cargo.lock|Cargo.toml|deny.toml|rust-toolchain.toml)
      if [ ! -f "$package_entry" ] || [ -L "$package_entry" ]; then
        printf '%s\n' \
          "api-surface package entry has an unexpected type: packages/$entry_name" >&2
        exit 1
      fi
      continue
      ;;
    .cargo|.config|policy-inputs|target)
      if [ ! -d "$package_entry" ] || [ -L "$package_entry" ]; then
        printf '%s\n' \
          "api-surface package entry has an unexpected type: packages/$entry_name" >&2
        exit 1
      fi
      continue
      ;;
  esac

  if [ -d "$package_entry" ] && [ ! -L "$package_entry" ] \
    && [ -f "$package_entry/Cargo.toml" ] \
    && [ ! -L "$package_entry/Cargo.toml" ]; then
    independent_root="packages/$entry_name"
    if grep -Eq '^[[:space:]]*\[workspace\][[:space:]]*$' \
        "$package_entry/Cargo.toml" \
      && is_independent_workspace_root "$independent_root"; then
      continue
    fi
  fi
  printf '%s\n' \
    "api-surface package entry is not a workspace member or classified generated/independent directory: packages/$entry_name" >&2
  exit 1
done <"$enumeration_file"

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
