#!/usr/bin/env bash
set -euo pipefail

command_name="${1:?usage: generated-artifact-check.sh <xtask-subcommand>}"
runfiles_root="${TEST_SRCDIR:?}/${TEST_WORKSPACE:?}"
xtask="$runfiles_root/${D2B_XTASK_RUNFILE:?}"
python="$runfiles_root/${D2B_PYTHON_RUNFILE:?}"
baseline="${TEST_TMPDIR:?}/${command_name}-baseline"
working="${TEST_TMPDIR:?}/${command_name}-working"

mkdir -p "$baseline" "$working"
while IFS= read -r -d '' source; do
  relative="${source#"$runfiles_root"/}"
  case "$relative" in
    packages/xtask/xtask | tests/tools/generated-artifact-check.sh) continue ;;
  esac
  destination="$baseline/$relative"
  mkdir -p "$(dirname "$destination")"
  cp -L -- "$source" "$destination"
done < <(find -L "$runfiles_root" -type f -print0)

cp -a "$baseline/." "$working/"
D2B_REPO_ROOT="$working" "$xtask" "$command_name" >/dev/null

if ! "$python" - "$baseline" "$working" <<'PY'
import filecmp
import pathlib
import sys

baseline = pathlib.Path(sys.argv[1])
working = pathlib.Path(sys.argv[2])

baseline_files = {
    path.relative_to(baseline)
    for path in baseline.rglob("*")
    if path.is_file()
}
working_files = {
    path.relative_to(working)
    for path in working.rglob("*")
    if path.is_file()
}

changed = sorted(
    (baseline_files ^ working_files)
    | {
        path
        for path in baseline_files & working_files
        if not filecmp.cmp(baseline / path, working / path, shallow=False)
    }
)

for path in changed:
    print(path)

raise SystemExit(bool(changed))
PY
then
  echo "$command_name generated artifacts drifted" >&2
  exit 1
fi
