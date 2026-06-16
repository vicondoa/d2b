#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
DEFAULT_PINNED_DIR="$ROOT/tests/golden/pinned"

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

export CARGO_BUILD_RUSTC_WRAPPER=${CARGO_BUILD_RUSTC_WRAPPER:-}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

pinned_inputs=("$@")
if [ "${#pinned_inputs[@]}" -eq 0 ]; then
  pinned_inputs=("$DEFAULT_PINNED_DIR")
fi

pinned_files=()
for input in "${pinned_inputs[@]}"; do
  if [ -d "$input" ]; then
    shopt -s nullglob
    dir_files=("$input"/*.txt)
    shopt -u nullglob
    pinned_files+=("${dir_files[@]}")
  elif [ -f "$input" ]; then
    pinned_files+=("$input")
  else
    echo "assert-pinned-tests: missing pinned test list: $input" >&2
    exit 1
  fi
done

if [ "${#pinned_files[@]}" -eq 0 ]; then
  echo "assert-pinned-tests: no pinned test list files found" >&2
  exit 1
fi

declare -A present
collect_present() {
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    present["${line#* }"]=1
  done
}
# Main workspace (packages/Cargo.toml).
collect_present < <(
  cd "$ROOT/packages"
  cargo nextest list --workspace --exclude nixling-contract-tests --message-format oneline
)
# Broker workspace (packages/nixling-priv-broker/Cargo.toml) is a SEPARATE
# cargo workspace, excluded from the main one. Retired canaries pinned
# ops::device / ops::modprobe #[test]s that live there, so the fail-closed
# pinned gate must enumerate it too — otherwise those retirements would be
# silently unguarded against deletion.
#
# `cargo metadata --all-features` (run by `nextest list`) can add a
# transitive lock entry the committed lock omits (e.g. `itoa` under rustix's
# full feature set), which would dirty the working tree. Snapshot + restore
# the broker lock so listing is non-mutating by construction.
broker_lock="$ROOT/packages/nixling-priv-broker/Cargo.lock"
broker_lock_backup=""
if [ -f "$broker_lock" ]; then
  broker_lock_backup=$(mktemp)
  cp "$broker_lock" "$broker_lock_backup"
fi
collect_present < <(
  cd "$ROOT/packages/nixling-priv-broker"
  # `--features layer1-bootstrap,fake-backends` lists a SUPERSET of the broker
  # test surface: the default real-wire tests, the layer1-bootstrap legacy
  # probe-* + scm_rights_fd_lifecycle fd-passing tests, AND the
  # `#![cfg(feature = "fake-backends")]`-gated hermetic integration tests
  # (e.g. tests/pidfd_handoff_scm_rights.rs). rust-workspace-checks.sh runs the
  # default, layer1-bootstrap, AND fake-backends broker test passes, so every
  # listed test is actually executed and can be guarded by the pinned gate.
  cargo nextest list --workspace --features layer1-bootstrap,fake-backends --message-format oneline
)
if [ -n "$broker_lock_backup" ]; then
  cp "$broker_lock_backup" "$broker_lock"
  rm -f "$broker_lock_backup"
fi

declare -A seen
total=0
missing=0
duplicates=0
for pinned_file in "${pinned_files[@]}"; do
  while IFS= read -r pinned || [ -n "$pinned" ]; do
    case "$pinned" in
      ""|\#*) continue ;;
    esac
    total=$((total + 1))
    if [ "${seen[$pinned]+set}" = set ]; then
      echo "assert-pinned-tests: duplicate pinned test: $pinned ($pinned_file)" >&2
      duplicates=$((duplicates + 1))
      continue
    fi
    seen["$pinned"]=1
    if [ "${present[$pinned]+set}" != set ]; then
      echo "assert-pinned-tests: missing pinned test: $pinned ($pinned_file)" >&2
      missing=$((missing + 1))
    fi
  done < "$pinned_file"
done

if [ "$total" -eq 0 ]; then
  echo "assert-pinned-tests: no pinned tests found in ${pinned_files[*]}" >&2
  exit 1
fi

if [ "$missing" -ne 0 ] || [ "$duplicates" -ne 0 ]; then
  echo "assert-pinned-tests: failed ($missing missing, $duplicates duplicate, $total pinned)" >&2
  exit 1
fi

echo "assert-pinned-tests: all $total pinned tests present (${#pinned_files[@]} file(s))"
