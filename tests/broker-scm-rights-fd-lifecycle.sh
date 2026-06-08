#!/usr/bin/env bash
# SCM_RIGHTS fd plumbing stays leak-free under fakes.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
MANIFEST=${MANIFEST:-$ROOT/packages/nixling-priv-broker/Cargo.toml}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_BROKER_FD_LIFECYCLE_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "broker-scm-rights-fd-lifecycle: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_BROKER_FD_LIFECYCLE_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir broker)}

cargo test --manifest-path "$MANIFEST" --features layer1-bootstrap scm_rights_fd_lifecycle -- --test-threads=1 --nocapture
printf 'broker-scm-rights-fd-lifecycle: PASS\n'
