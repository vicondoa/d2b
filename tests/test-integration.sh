#!/usr/bin/env bash
# tests/test-integration.sh — `make test-integration`: Layer-2 podman container
# integration tests. Each tests/integration/containers/*.sh builds its Nix-built
# OCI image (containerImages.<system>.<name>, NOT swept by `nix flake check`) and
# runs it with rootless podman. Scope is foreign-userland portability only (e.g.
# a static d2b binary on stock Ubuntu); daemon/socket activation is covered
# natively. Runs identically on a NixOS host and an ubuntu-latest runner.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}
export ROOT D2B_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

mapfile -t scripts < <(
  find tests/integration/containers -maxdepth 1 -name '*.sh' ! -name 'lib.sh' -type f 2>/dev/null | sort
)
if [ "${#scripts[@]}" -eq 0 ]; then
  log "test-integration: no tests/integration/containers/*.sh runners present"
  exit 0
fi

rc=0
for s in "${scripts[@]}"; do
  log "==> $s"
  if bash "$s"; then
    ok "$s"
  else
    fail "$s"
    rc=1
  fi
done

[ "$rc" -eq 0 ] || exit 1
log "test-integration OK"
