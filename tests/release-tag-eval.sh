#!/usr/bin/env bash
# v1.x invariant gate: assert release-tag shape for the release tags.
#
# Default: validates `refs/tags/v1.1`.
# Override with `--tag-ref refs/tags/<ref>` for CI dry-run or rc tags.
#
# Asserts:
#   (a) tag is ANNOTATED (`git cat-file -t <ref>` returns `tag`),
#       NOT lightweight (which returns `commit`).
#   (b) tag points at a real commit (resolvable).
#   (c) tag message names the release: its first non-empty line equals
#       the tag's version token (e.g. `refs/tags/v1.1` -> `v1.1`,
#       `refs/tags/v1.1-rc2` -> `v1.1-rc2`). This matches the actual
#       release-tag convention: the shipped v1.0/v1.1/v1.2 annotated
#       tags carry the bare version string as their message.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

tag_ref="refs/tags/v1.1"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag-ref) tag_ref="$2"; shift 2 ;;
    *) printf 'release-tag-eval: usage: %s [--tag-ref refs/tags/<ref>]\n' "$0" >&2; exit 2 ;;
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

# (c) tag message names the release: first non-empty, trailing-trimmed
# line equals the version token. Matches the shipped convention where
# the annotated v1.0/v1.1/v1.2 tags carry the bare version as message.
version=${tag_ref#refs/tags/}
message=$(git tag -l --format='%(contents)' "$version" 2>/dev/null || true)
first_line=$(printf '%s\n' "$message" | sed -e 's/[[:space:]]*$//' -e '/^$/d' | head -n1)
if [ "$first_line" = "$version" ]; then
  printf 'release-tag-eval: PASS (%s -> %s; tag message names the release "%s")\n' "$tag_ref" "$commit" "$version"
  exit 0
fi

printf 'release-tag-eval: FAIL — %s message first line "%s" does not name the release "%s"\n' "$tag_ref" "$first_line" "$version" >&2
printf '  tag message:\n%s\n' "$message" >&2
exit 1
