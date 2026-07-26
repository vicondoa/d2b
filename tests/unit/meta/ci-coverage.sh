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

checkout_credential_errors=()
while IFS= read -r workflow; do
  checkout_count=$(grep -Ec 'uses:[[:space:]]*actions/checkout@' "$workflow" || true)
  secured_count=$(
    awk '
      /^[[:space:]]*- uses:[[:space:]]*actions\/checkout@/ {
        if (getline with_line <= 0) {
          next
        }
        sub(/^[[:space:]]*/, "", with_line)
        if (with_line != "with:") {
          next
        }
        if (getline credential_line <= 0) {
          next
        }
        sub(/^[[:space:]]*/, "", credential_line)
        if (credential_line == "persist-credentials: false") {
          secured += 1
        }
      }
      END { print secured + 0 }
    ' "$workflow"
  )
  if [ "$checkout_count" -ne "$secured_count" ]; then
    checkout_credential_errors+=(
      "${workflow#"$ROOT"/}: $secured_count of $checkout_count checkout steps immediately disable credential persistence"
    )
  fi
done < <(
  find "$WORKFLOW_DIR" -maxdepth 1 -type f \
    \( -name '*.yml' -o -name '*.yaml' \) -print \
    | LC_ALL=C sort
)

if [ "${#checkout_credential_errors[@]}" -gt 0 ]; then
  echo "FAIL: GitHub Actions checkout credential coverage is incomplete:" >&2
  printf '  %s\n' "${checkout_credential_errors[@]}" >&2
  exit 1
fi

expected_workflow_shell='shell: bash tests/tools/ci-shell {0}'
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
# A gate that can exit 0 without doing its work must say so in the manifest.
#
# Reachability alone is not coverage: a gate wired to a manifest target still
# contributes a green result while skipping. Every gate guarded by an opt-in
# environment variable must therefore belong to a job declared
# "enforcement": "advisory", so no caller can count it as an enforcing pass.
# ---------------------------------------------------------------------------
undeclared_skippable=()
while IFS= read -r gate; do
  rel=${gate#"$ROOT"/}
  # A skip guard is an early, unconditional `exit 0` reached when an opt-in
  # variable is unset. Match the guard shape rather than any particular name.
  if ! grep -qE '^\s*if \[ "\$\{D2B_[A-Z_]+:-0\}" != 1 \]' "$gate"; then
    continue
  fi
  target=$(basename "$gate" .sh)
  if ! grep -q "\"makeTarget\": \"test-${target}\"" "$MANIFEST"; then
    continue
  fi
  if ! awk -v t="test-${target}" '
    $0 ~ "\"makeTarget\": \"" t "\"" { found = 1 }
    found && /"enforcement": "advisory"/ { ok = 1 }
    found && /^    },/ { exit }
    END { exit ok ? 0 : 1 }
  ' "$MANIFEST"; then
    undeclared_skippable+=("$rel")
  fi
done < <(
  find "$ROOT/tests/unit/gates" -maxdepth 1 -type f -name '*.sh' -print \
    | LC_ALL=C sort
)

if [ ${#undeclared_skippable[@]} -ne 0 ]; then
  echo "FAIL: gates that can skip are not declared advisory in the manifest:" >&2
  for g in "${undeclared_skippable[@]}"; do
    echo "  $g" >&2
  done
  echo "" >&2
  echo "Remediation: give the owning job \"enforcement\": \"advisory\" so a skipped" >&2
  echo "run is not reported as an enforcing pass, or remove the skip guard." >&2
  exit 1
fi

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
