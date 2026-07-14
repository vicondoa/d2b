#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
DEFAULT_PINNED_DIR="$ROOT/tests/golden/pinned"

# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

workspace_target_dir=${D2B_WORKSPACE_GATE_TARGET_DIR:-$(d2b_cargo_target_dir workspace)}

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
  if [ -z "${D2B_ASSERT_PINNED_IN_NIX_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    export D2B_ASSERT_PINNED_IN_NIX_SHELL=1
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
    present["$line"]=1
    present["${line#* }"]=1
  done
}
# Main workspace (packages/Cargo.toml).
collect_present < <(
  cd "$ROOT/packages"
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo nextest list --locked --workspace --exclude d2b-contract-tests --message-format oneline
)
# Fixture contract tests are excluded from the default workspace test pass, but
# test-rust.sh runs them with D2B_FIXTURES. Include their nextest
# listing so retired shell gates can pin rendered-artifact contract successors.
collect_present < <(
  cd "$ROOT/packages"
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo nextest list --locked -p d2b-contract-tests --message-format oneline
)
# Broker package feature tests are part of the unified workspace. Retired canaries pinned
# ops::device / ops::modprobe #[test]s that live there, so the fail-closed
# pinned gate must enumerate it too — otherwise those retirements would be
# silently unguarded against deletion.
collect_present < <(
  cd "$ROOT/packages"
  # `--features layer1-bootstrap,fake-backends` lists a SUPERSET of the broker
  # test surface: the default real-wire tests, the layer1-bootstrap legacy
  # probe-* + scm_rights_fd_lifecycle fd-passing tests, AND the
  # `#![cfg(feature = "fake-backends")]`-gated hermetic integration tests
  # (e.g. tests/pidfd_handoff_scm_rights.rs). test-rust.sh runs the
  # default, layer1-bootstrap, AND fake-backends broker test passes, so every
  # listed test is actually executed and can be guarded by the pinned gate.
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo nextest list --locked -p d2b-priv-broker \
      --features d2b-priv-broker/layer1-bootstrap,d2b-priv-broker/fake-backends \
      --message-format oneline
)

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
