#!/usr/bin/env bash
# Publish the current capture set into a fresh timestamped run directory in the
# review repo, so the newest screenshots are always unambiguous.
#
#   ./publish-review.sh [note]
set -euo pipefail

LAB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$LAB_ROOT/out"
REVIEW="${REVIEW_REPO:-$HOME/projects/d2b-ux-screenshot-review}"
NOTE="${1:-}"
BUDGET=$((5 * 1024 * 1024))

[[ -d "$REVIEW/.git" ]] || { echo "no review repo at $REVIEW" >&2; exit 1; }

RUN="$(date -u +%Y-%m-%d_%H%M%S)"
DEST="$REVIEW/runs/$RUN"
mkdir -p "$DEST/sheets" "$DEST/live"

shopt -s nullglob
cp "$OUT"/sheets/*.png "$DEST/sheets/" 2>/dev/null || true
for f in "$OUT"/live-*.png "$OUT"/tab-*.png "$OUT"/wall-*.png \
         "$OUT"/detail-*.png "$OUT"/clip-*.png "$OUT"/zoom-*.png; do
  cp "$f" "$DEST/live/"
done
shopt -u nullglob

# Fail closed on the image budget rather than pushing something unviewable.
over=0
while IFS= read -r -d '' f; do
  bytes="$(stat -c %s "$f")"
  if (( bytes > BUDGET )); then
    echo "OVER BUDGET: $f (${bytes}B)" >&2
    over=1
  fi
done < <(find "$DEST" -name '*.png' -print0)
(( over == 0 )) || exit 1

( cd "$DEST" && find . -name '*.png' | sort | xargs sha256sum > SHA256SUMS )
( cd "$DEST" && find . -name '*.png' | sort | while read -r f; do
    printf '%s\t%s\n' "$(stat -c %s "$f")" "$f"
  done > SIZES.tsv )

{
  printf '# Run %s\n\n' "$RUN"
  [[ -n "$NOTE" ]] && printf '%s\n\n' "$NOTE"
  printf 'Captured %s UTC.\n\n' "$(date -u +'%Y-%m-%d %H:%M:%S')"
  printf 'Commit: %s\n\n' "$(cd "$LAB_ROOT/.." && git rev-parse --short HEAD)"
  printf '| File | Bytes |\n| --- | --- |\n'
  ( cd "$DEST" && find . -name '*.png' | sort | while read -r f; do
      printf '| `%s` | %s |\n' "${f#./}" "$(stat -c %s "$f")"
    done )
} > "$DEST/README.md"

# A stable pointer to the newest run.
printf '%s\n' "$RUN" > "$REVIEW/LATEST_RUN"

echo "published $DEST"
echo "files: $(find "$DEST" -name '*.png' | wc -l)"
