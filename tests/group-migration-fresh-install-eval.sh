#!/usr/bin/env bash
# Phase B fresh-install gate: the legacy-gid migration activation block
# must be present, guarded by root existence checks, and must not use a
# path-based chgrp fallback when no legacy state exists.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
activation_module="$ROOT/nixos-modules/host-activation.nix"
helper_main="$ROOT/nixos-modules/host-activation-helper/src/main.rs"

if [ ! -f "$activation_module" ] || [ ! -f "$helper_main" ]; then
  printf 'group-migration-fresh-install-eval: FAIL — migration module/helper missing\n' >&2
  exit 1
fi

require_literal() {
  local needle="$1" file="$2"
  if ! grep -F -- "$needle" "$file" >/dev/null 2>&1; then
    printf 'group-migration-fresh-install-eval: FAIL — missing %s in %s\n' "$needle" "$file" >&2
    exit 1
  fi
}

require_literal 'system.activationScripts.nixlingGroupMigration' "$activation_module"
require_literal 'lib.stringAfter [ "users" ]' "$activation_module"
require_literal '[ -e "$root" ] || continue' "$activation_module"
require_literal 'chgrp-by-numeric-gid' "$activation_module"
require_literal '--skip-while-lock-held /run/nixling/daemon.lock' "$activation_module"
require_literal 'lib.stringAfter [ "users" "nixlingGroupMigration" ]' "$activation_module"
require_literal 'O_DIRECTORY | libc::O_NOFOLLOW' "$helper_main"
require_literal 'libc::F_OFD_SETLK' "$helper_main"
require_literal 'libc::AT_SYMLINK_NOFOLLOW' "$helper_main"

if awk '
    /system.activationScripts.nixlingGroupMigration/ { in_block=1 }
    in_block { print }
    in_block && /^    '\'''\'';/ { exit }
  ' "$activation_module" \
    | grep -E '(^|[[:space:]])chgrp([[:space:]]|$)' >/dev/null 2>&1; then
  printf 'group-migration-fresh-install-eval: FAIL — raw chgrp found in migration module\n' >&2
  exit 1
fi

printf 'group-migration-fresh-install-eval: PASS\n'
