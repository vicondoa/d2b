#!/usr/bin/env bash
# v1.1 invariant gate: assert release-tag shape for the
# upcoming v1.1 (and v1.1-rcN) tags.
#
# Default: validates `refs/tags/v1.1`.
# Override with `--tag-ref refs/tags/<ref>` for CI dry-run or rc tags.
#
# Asserts:
#   (a) tag is ANNOTATED (`git cat-file -t <ref>` returns `tag`),
#       NOT lightweight (which returns `commit`).
#   (b) tag points at a real commit (resolvable).
#   (c) tag message contains the literal substring
#       `9/9 unanimous panel signoff` OR (for rc tags) the
#       substring `v1.1-rcN panel signoff` per the rc convention.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

tag_ref="refs/tags/v1.1"
allow_rc=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag-ref) tag_ref="$2"; shift 2 ;;
    --allow-rc) allow_rc=1; shift ;;
    *) printf 'release-tag-eval: usage: %s [--tag-ref refs/tags/<ref>] [--allow-rc]\n' "$0" >&2; exit 2 ;;
  esac
done

cd "$ROOT"

# (a) annotated check
type=$(git cat-file -t "$tag_ref" 2>/dev/null || true)
if [ "$type" != "tag" ]; then
  printf 'release-tag-eval: FAIL — %s is not annotated (got: %s; lightweight tags rejected)\n' "$tag_ref" "${type:-missing}" >&2
  exit 1
fi

# (b) resolvable commit
commit=$(git rev-parse --verify "$tag_ref^{commit}" 2>/dev/null || true)
if [ -z "$commit" ]; then
  printf 'release-tag-eval: FAIL — %s does not resolve to a commit\n' "$tag_ref" >&2
  exit 1
fi

# (c) signoff message check
message=$(git tag -l --format='%(contents)' "${tag_ref#refs/tags/}" 2>/dev/null || true)
if printf '%s' "$message" | grep -q '9/9 unanimous panel signoff'; then
  printf 'release-tag-eval: PASS (%s -> %s; final v1.1 signoff message present)\n' "$tag_ref" "$commit"
  exit 0
fi
if [ "$allow_rc" = 1 ] && printf '%s' "$message" | grep -qE 'v1\.1-rc[0-9]+ panel signoff'; then
  printf 'release-tag-eval: PASS (%s -> %s; rc panel signoff message present)\n' "$tag_ref" "$commit"
  exit 0
fi

printf 'release-tag-eval: FAIL — %s message missing required signoff substring\n' "$tag_ref" >&2
printf '  tag message:\n%s\n' "$message" >&2
exit 1
