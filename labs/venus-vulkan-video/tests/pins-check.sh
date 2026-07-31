#!/usr/bin/env bash
# Assert PINS.md records the revisions flake.lock actually pins.
#
# PINS.md exists so a capability report or benchmark in docs/ can be traced to
# the exact code that produced it. A stale entry does not fail loudly -- it
# silently attributes a measurement to the wrong revision, which is worse than
# having no manifest at all. So the manifest is checked rather than trusted.
#
# Usage:
#   pins-check.sh [--fix] [<lab-dir>]
#
# --fix rewrites the fork revisions in PINS.md from flake.lock instead of
# failing, for use after a deliberate pin bump.

set -euo pipefail

LAB_DIR=""
FIX=0
for arg in "$@"; do
  case $arg in
    --fix) FIX=1 ;;
    *) LAB_DIR=$arg ;;
  esac
done

if [ -z "$LAB_DIR" ]; then
  LAB_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fi

LOCK="$LAB_DIR/flake.lock"
PINS="$LAB_DIR/PINS.md"

die() { echo "pins-check: $*" >&2; exit 1; }

[ -f "$LOCK" ] || die "no flake.lock at $LOCK"
[ -f "$PINS" ] || die "no PINS.md at $PINS"

# Pull "<input> <repo> <rev>" for every locked node, without a JSON parser: the
# lock is machine-generated with stable formatting, and adding a jq dependency
# to a check this small is not worth it.
#
# Keyed on the input NAME rather than the repo, because each fork is locked
# twice -- once at its working revision (`*-src`) and once at the upstream
# commit it was seeded from (`*-base`) -- and PINS.md records both.
mapfile -t triples < <(
  awk '
    /^    "[a-z0-9-]+": \{$/ {
      n = $0; sub(/^ *"/, "", n); sub(/": \{$/, "", n); node = n
    }
    /"repo": "/ { r = $0; sub(/.*"repo": "/, "", r); sub(/".*/, "", r); repo = r }
    /"rev": "/  { v = $0; sub(/.*"rev": "/,  "", v); sub(/".*/, "", v);
                  if (repo != "") { print node, repo, v; repo = "" } }
  ' "$LOCK" | sort -u
)

[ "${#triples[@]}" -gt 0 ] || die "found no locked revisions in $LOCK"

status=0
for triple in "${triples[@]}"; do
  read -r input repo rev <<< "$triple"

  case $repo in *-vulkan-video) ;; *) continue ;; esac

  # PINS.md abbreviates revisions, so match on the prefix actually written.
  line=$(grep -F "\`vicondoa/$repo\`" "$PINS" || true)
  if [ -z "$line" ]; then
    echo "pins-check: PINS.md has no row for vicondoa/$repo" >&2
    status=1
    continue
  fi

  case $input in
    *-base) recorded=$(printf '%s' "$line" | sed -n 's/.*base `\([0-9a-f]\{7,\}\)`.*/\1/p') ;;
    *)      recorded=$(printf '%s' "$line" | sed -n 's/.*| `\([0-9a-f]\{7,\}\)` (W.*/\1/p') ;;
  esac

  if [ -z "$recorded" ]; then
    echo "pins-check: could not read the $input revision from the $repo row" >&2
    status=1
    continue
  fi

  if [ "${rev:0:${#recorded}}" = "$recorded" ]; then
    continue
  fi

  if [ "$FIX" = 1 ]; then
    short=${rev:0:${#recorded}}
    sed -i "s|\(\`vicondoa/$repo\`.*\)\`$recorded\`|\1\`$short\`|" "$PINS"
    echo "pins-check: updated $input $recorded -> $short"
  else
    echo "pins-check: $input pinned at $rev but PINS.md records $recorded" >&2
    status=1
  fi
done

if [ "$status" != 0 ]; then
  echo >&2
  echo "  PINS.md is the manifest that ties every measurement in docs/ to the" >&2
  echo "  code that produced it. Rerun with --fix after a deliberate bump." >&2
  exit 1
fi

echo "pins-check: PINS.md matches flake.lock"
