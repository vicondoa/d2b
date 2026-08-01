#!/usr/bin/env bash
# Reclaim this worktree's disk: cargo target directories, the scratch tree,
# and unreferenced Nix store paths.
#
# Deliberately NOT removed:
#   - the shared sccache directory ($SCCACHE_DIR, default
#     ~/.cache/d2b-sccache). It is what keeps a rebuild after this target
#     cheap, and it is shared across every worktree.
#   - anything outside this worktree. Sibling worktrees own their own
#     artifacts and may have work in flight.
#
# Environment knobs:
#   D2B_CLEAN_SKIP_GC=1        skip nix-collect-garbage
#   D2B_CLEAN_KEEP_SCRATCH=1   keep .scratch/
#   D2B_CLEAN_DRY_RUN=1        report what would be removed, remove nothing

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

dry_run="${D2B_CLEAN_DRY_RUN:-0}"
total_bytes=0

log() { printf '%s\n' "$*"; }

# A path is safe to remove only when it sits inside this worktree AND holds
# no git-tracked file. The tracked-file check is the load-bearing guard: it
# is what makes an unexpected match (a source directory that happens to be
# named "target", a future layout change) fail closed rather than delete
# committed content.
assert_removable() {
  local path="$1" resolved
  resolved="$(readlink -f -- "$path")"

  case "$resolved" in
    "$ROOT"/?*) : ;;
    *)
      printf 'clean: refusing to remove %s: outside %s\n' "$resolved" "$ROOT" >&2
      exit 1
      ;;
  esac

  if [ -n "$(git ls-files -- "$path")" ]; then
    printf 'clean: refusing to remove %s: contains git-tracked files\n' "$path" >&2
    exit 1
  fi
}

remove_path() {
  local path="$1" bytes
  [ -e "$path" ] || return 0
  assert_removable "$path"

  bytes="$(du -sb -- "$path" 2>/dev/null | cut -f1)"
  [ -n "$bytes" ] || bytes=0
  total_bytes=$((total_bytes + bytes))

  if [ "$dry_run" = "1" ]; then
    log "  would remove $(numfmt --to=iec --suffix=B -- "$bytes")	${path#./}"
    return 0
  fi

  log "  removing $(numfmt --to=iec --suffix=B -- "$bytes")	${path#./}"
  rm -rf -- "$path"
}

log "clean: cargo target directories"
# A directory named `target` counts as a cargo target directory when it sits
# beside a Cargo.toml or carries cargo's own CACHEDIR.TAG marker. Nested
# target directories are pruned: removing the outer one takes them with it.
#
# .scratch is excluded here and handled as a single unit below. The warm test
# caches under it are target directories, but they live and die with the
# scratch tree - counting them separately would double-count the reclaim, and
# removing them individually would gut the caches that
# D2B_CLEAN_KEEP_SCRATCH exists to preserve.
while IFS= read -r dir; do
  [ -f "${dir%/target}/Cargo.toml" ] || [ -f "$dir/CACHEDIR.TAG" ] || continue
  remove_path "$dir"
done < <(find . -type d \( -path ./.git -o -path ./.scratch \) -prune -o \
  -type d -name target -prune -print | sort)

if [ "${D2B_CLEAN_KEEP_SCRATCH:-0}" = "1" ]; then
  log "clean: keeping .scratch (D2B_CLEAN_KEEP_SCRATCH=1)"
else
  log "clean: scratch tree"
  remove_path ./.scratch
fi

log "clean: reclaimed $(numfmt --to=iec --suffix=B -- "$total_bytes") from this worktree"

if [ "${D2B_CLEAN_SKIP_GC:-0}" = "1" ]; then
  log "clean: skipping nix-collect-garbage (D2B_CLEAN_SKIP_GC=1)"
  exit 0
fi

if [ "$dry_run" = "1" ]; then
  log "clean: would run nix-collect-garbage"
  exit 0
fi

if ! command -v nix-collect-garbage >/dev/null 2>&1; then
  log "clean: nix-collect-garbage not on PATH, skipping store collection"
  exit 0
fi

# User-scoped collection only. Deleting old *system* generations needs sudo
# and is operator policy rather than a repository target; AGENTS.md documents
# `sudo nix-collect-garbage --delete-older-than 7d` for that.
log "clean: collecting unreferenced Nix store paths"
nix-collect-garbage
