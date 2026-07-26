#!/usr/bin/env bash
# tests/unit/meta/ci-coverage.sh - structural gate asserting every closed-set
# shell gate is reachable from a Make target in the local Layer-1 manifest.
#
# Root-cause gap: static CI set drift.
#
# Exits 0 if all gates are covered, 1 if any are orphaned.
#
# Usage:
#   bash tests/unit/meta/ci-coverage.sh

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

MANIFEST="$ROOT/tests/layer1-jobs.json"
MAKEFILE="$ROOT/Makefile"
WORKFLOW_DIR="$ROOT/.github/workflows"

for required in "$MANIFEST" "$MAKEFILE"; do
  if [ ! -f "$required" ]; then
    echo "ERROR: missing coverage source $required" >&2
    exit 1
  fi
done

expected_workflow_shell='shell: ./tests/tools/scrub-shell-environment -c '\''exec bash "$@"'\'' d2b-ci {0}'
workflow_shell_errors=()
while IFS= read -r workflow; do
  found=0
  while IFS= read -r line; do
    found=1
    trimmed=${line#"${line%%[![:space:]]*}"}
    if [ "$trimmed" != "$expected_workflow_shell" ]; then
      workflow_shell_errors+=("${workflow#"$ROOT"/}: unsupported shell declaration: $trimmed")
    fi
  done < <(grep -E '^[[:space:]]*shell:' "$workflow" || true)
  if [ "$found" -eq 0 ]; then
    workflow_shell_errors+=("${workflow#"$ROOT"/}: missing scrubbed default run shell")
  fi
done < <(find "$WORKFLOW_DIR" -maxdepth 1 -type f -name '*.yml' -print | LC_ALL=C sort)

if [ "${#workflow_shell_errors[@]}" -gt 0 ]; then
  echo "FAIL: GitHub Actions workflow shell coverage is incomplete:" >&2
  printf '  %s\n' "${workflow_shell_errors[@]}" >&2
  exit 1
fi

mapfile -t local_job_ids < <(
  awk '
    /^[[:space:]]*"local":[[:space:]]*\{/ { in_local = 1; next }
    /^[[:space:]]*"ci":[[:space:]]*\{/ { in_local = 0 }
    in_local && /"jobs":[[:space:]]*\[/ {
      line = $0
      sub(/^.*\[/, "", line)
      sub(/\].*$/, "", line)
      while (match(line, /"[^"]+"/)) {
        print substr(line, RSTART + 1, RLENGTH - 2)
        line = substr(line, RSTART + RLENGTH)
      }
    }
  ' "$MANIFEST"
)

if [ "${#local_job_ids[@]}" -eq 0 ]; then
  echo "ERROR: Layer-1 manifest has no local jobs" >&2
  exit 1
fi

manifest_targets=()
for job_id in "${local_job_ids[@]}"; do
  target=$(
    awk -v job_id="$job_id" '
      $0 ~ "^[[:space:]]*\"" job_id "\":[[:space:]]*\\{" { in_job = 1; next }
      in_job && /"makeTarget":[[:space:]]*"/ {
        line = $0
        sub(/^.*"makeTarget":[[:space:]]*"/, "", line)
        sub(/".*$/, "", line)
        print line
        exit
      }
      in_job && /^[[:space:]]*"[^"]+":[[:space:]]*\{/ { exit }
    ' "$MANIFEST"
  )
  if [ -z "$target" ]; then
    echo "ERROR: local manifest job $job_id has no makeTarget" >&2
    exit 1
  fi
  manifest_targets+=("$target")
done

entrypoint_scripts=()
for target in "${manifest_targets[@]}"; do
  recipe=$(
    awk -v target="$target" '
      $0 ~ "^" target "([[:space:]]*:[^=]*)?$" { in_target = 1; found = 1; next }
      in_target && /^\t/ { print; next }
      in_target && /^[^[:space:]#]/ { exit }
      END { if (!found) exit 2 }
    ' "$MAKEFILE"
  ) || {
    echo "ERROR: manifest target $target is not defined in Makefile" >&2
    exit 1
  }
  while IFS= read -r script; do
    entrypoint_scripts+=("$script")
  done < <(
    printf '%s\n' "$recipe" \
      | grep -oE '(tests|scripts)/[A-Za-z0-9_./-]+\.sh' \
      | LC_ALL=C sort -u \
      || true
  )
done

is_direct_entrypoint() {
  local rel="$1"
  local entrypoint
  for entrypoint in "${entrypoint_scripts[@]}"; do
    [ "$entrypoint" = "$rel" ] && return 0
  done
  return 1
}

is_referenced_by_entrypoint() {
  local rel="$1"
  local entrypoint
  for entrypoint in "${entrypoint_scripts[@]}"; do
    [ -f "$ROOT/$entrypoint" ] || continue
    if awk -v rel="$rel" '
      /^[[:space:]]*#/ { next }
      index($0, rel) { found = 1 }
      END { exit found ? 0 : 1 }
    ' "$ROOT/$entrypoint"; then
      return 0
    fi
  done
  return 1
}

orphans=()
covered=0

while IFS= read -r script; do
  rel=${script#"$ROOT"/}
  if is_direct_entrypoint "$rel" || is_referenced_by_entrypoint "$rel"; then
    covered=$((covered + 1))
  else
    orphans+=("$rel")
  fi
done < <(
  find "$ROOT/tests/unit/meta" "$ROOT/tests/unit/gates" \
    -maxdepth 1 -type f -name '*.sh' -print \
    | LC_ALL=C sort
)

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
if [ ${#orphans[@]} -eq 0 ]; then
  echo "PASS: $covered closed-set shell gates reachable from local Layer-1 manifest targets"
  exit 0
else
  echo "FAIL: closed-set shell gates not reachable from a local Layer-1 manifest target:" >&2
  for o in "${orphans[@]}"; do
    echo "  $o" >&2
  done
  echo "" >&2
  echo "Remediation: wire each orphan through a manifest-listed Make target or its direct driver." >&2
  echo "FAIL: $covered gates reachable; ${#orphans[@]} orphaned" >&2
  exit 1
fi
