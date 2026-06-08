#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
min_gib=${NL_MIN_DISK_GIB:-10}

case "$min_gib" in
  ''|*[!0-9]*)
    printf 'tests/preflight-disk-space: NL_MIN_DISK_GIB must be a whole number GiB\n' >&2
    exit 1
    ;;
esac

avail_kib=$(df --output=avail "$ROOT" | tail -n1 | tr -d '[:space:]')
min_kib=$((min_gib * 1024 * 1024))

if [ "$avail_kib" -gt "$min_kib" ]; then
  exit 0
fi

actual_gib=$(awk -v kib="$avail_kib" 'BEGIN { printf "%.1f", kib / 1048576 }')
cat >&2 <<EOF
tests/preflight-disk-space: free disk on $ROOT below ${min_gib} GiB (${actual_gib} GiB).
Remediation:
  1. nix-collect-garbage
  2. rm -rf $ROOT/.nl-* $ROOT/.static-* (or \`nl_reap_scratch_orphans\`)
  3. For multi-worktree dev: confirm packages/.cargo/config.toml points
     target-dir at the shared /home/paydro/.cache/nixling-cargo-target/
EOF
exit 1
