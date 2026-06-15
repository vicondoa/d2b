#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
PINNED_FILE=${1:-"$ROOT/tests/golden/argv-pinned-tests.txt"}

if ! command -v cargo >/dev/null 2>&1; then
  for candidate in "$HOME"/.rustup/toolchains/1.94.1-*/bin; do
    if [ -x "$candidate/cargo" ]; then
      PATH="$candidate:$PATH"
      export PATH
      break
    fi
  done
fi

if ! cargo nextest --version >/dev/null 2>&1; then
  if [ -z "${NIXLING_ASSERT_PINNED_IN_NIX_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    export NIXLING_ASSERT_PINNED_IN_NIX_SHELL=1
    exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-nextest nixpkgs#gcc \
      --command bash "$0" "$@"
  fi
  echo "assert-pinned-tests: cargo-nextest is required" >&2
  exit 1
fi

[ -f "$PINNED_FILE" ] || {
  echo "assert-pinned-tests: missing pinned test list: $PINNED_FILE" >&2
  exit 1
}

export CARGO_BUILD_RUSTC_WRAPPER=${CARGO_BUILD_RUSTC_WRAPPER:-}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

declare -A present
while IFS= read -r line; do
  [ -n "$line" ] || continue
  present["${line#* }"]=1
done < <(
  cd "$ROOT/packages"
  cargo nextest list --workspace --exclude nixling-contract-tests --message-format oneline
)

declare -A seen
total=0
missing=0
duplicates=0
while IFS= read -r pinned || [ -n "$pinned" ]; do
  case "$pinned" in
    ""|\#*) continue ;;
  esac
  total=$((total + 1))
  if [ "${seen[$pinned]+set}" = set ]; then
    echo "assert-pinned-tests: duplicate pinned test: $pinned" >&2
    duplicates=$((duplicates + 1))
    continue
  fi
  seen["$pinned"]=1
  if [ "${present[$pinned]+set}" != set ]; then
    echo "assert-pinned-tests: missing pinned test: $pinned" >&2
    missing=$((missing + 1))
  fi
done < "$PINNED_FILE"

if [ "$total" -eq 0 ]; then
  echo "assert-pinned-tests: no pinned tests found in $PINNED_FILE" >&2
  exit 1
fi

if [ "$missing" -ne 0 ] || [ "$duplicates" -ne 0 ]; then
  echo "assert-pinned-tests: failed ($missing missing, $duplicates duplicate, $total pinned)" >&2
  exit 1
fi

echo "assert-pinned-tests: all $total pinned argv tests present"
