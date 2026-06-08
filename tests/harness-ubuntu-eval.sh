#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
NL_LOG=${NL_LOG:-"$ROOT/.nixling-test.log"}
export NL_LOG

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

log "--> nix build harness-ubuntu-skeleton (x86_64-linux)"
nix build --no-link --print-build-logs ".#checks.x86_64-linux.harness-ubuntu-skeleton"
ok "harness-ubuntu-skeleton x86_64-linux"

log "--> harness-ubuntu-skeleton (aarch64-linux cross-eval)"
nix eval --raw ".#checks.aarch64-linux.harness-ubuntu-skeleton.drvPath" >/dev/null
ok "harness-ubuntu-skeleton aarch64-linux eval"
