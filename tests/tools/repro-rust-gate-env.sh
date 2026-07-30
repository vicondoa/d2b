#!/usr/bin/env bash
# tests/tools/repro-rust-gate-env.sh - reproduce ONE command inside the exact
# environment `make test-rust` builds for itself, without running the gate.
#
# The Rust gate re-enters a nix shell and provisions a private rustup toolchain
# before it runs anything. Failures that only appear inside that environment
# used to cost a full gate run to reproduce. This reconstructs the environment
# and hands it a single command, so the loop is a couple of minutes instead of
# half an hour.
#
#   bash tests/tools/repro-rust-gate-env.sh cargo test -p d2b-bus --test foo
#   bash tests/tools/repro-rust-gate-env.sh bash -c 'cargo doc -p d2b-notify'
#   bash tests/tools/repro-rust-gate-env.sh env            # dump the env
#
# The toolchain root is reused between invocations (D2B_REPRO_HOME, default
# .scratch/rust-gate-repro) so only the first run pays the rustup install.
# Pass --fresh to rebuild it.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

fresh=0
if [ "${1:-}" = "--fresh" ]; then
  fresh=1
  shift
fi

if [ "$#" -eq 0 ]; then
  echo "usage: $0 [--fresh] <command> [args...]" >&2
  exit 2
fi

toolchain_file="$ROOT/packages/rust-toolchain.toml"
pinned_channel=$(
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
)
if [ -z "$pinned_channel" ]; then
  echo "repro: could not read pinned Rust channel from $toolchain_file" >&2
  exit 1
fi

repro_home=${D2B_REPRO_HOME:-$ROOT/.scratch/rust-gate-repro}
if [ "$fresh" = 1 ] && [ -d "$repro_home" ]; then
  chmod -R u+w "$repro_home" 2>/dev/null || true
  rm -rf -- "$repro_home"
fi
mkdir -p "$repro_home"

export RUSTUP_HOME="$repro_home/rustup"
export CARGO_HOME="$repro_home/cargo"
export RUSTUP_TOOLCHAIN="$pinned_channel"

echo "repro: pinned channel $pinned_channel, toolchain root $repro_home" >&2
echo "repro: running: $*" >&2

# Mirror the gate's nix shell exactly: same inputs, same package set.
exec nix shell --quiet --inputs-from "$ROOT" \
  nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache \
  --command bash -c '
    set -euo pipefail
    if ! rustup toolchain list 2>/dev/null | grep -q "^${RUSTUP_TOOLCHAIN}"; then
      echo "repro: installing pinned toolchain (first run only)" >&2
      rustup toolchain install "$RUSTUP_TOOLCHAIN" \
        --profile minimal --component rustfmt --component clippy >&2
    fi
    export PATH="$CARGO_HOME/bin:$PATH"
    cd "$1/packages"
    shift
    exec "$@"
  ' _ "$ROOT" "$@"
