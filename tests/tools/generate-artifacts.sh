#!/usr/bin/env bash
set -euo pipefail

xtask_runfile="${1:?usage: generate-artifacts.sh <xtask-runfile> <command>...}"
shift
if (($# == 0)); then
  echo "generate-artifacts.sh: no generator commands supplied" >&2
  exit 2
fi

repo_root="${BUILD_WORKSPACE_DIRECTORY:?generate must run through bazel run}"
runfiles_root="${RUNFILES_DIR:-"$0.runfiles"}"
xtask=""
for root in "$runfiles_root" "$runfiles_root/_main" "${TEST_WORKSPACE:+$runfiles_root/$TEST_WORKSPACE}"; do
  if [[ -n "$root" && -x "$root/$xtask_runfile" ]]; then
    xtask="$root/$xtask_runfile"
    break
  fi
done
if [[ ! -x "$xtask" ]]; then
  echo "generate-artifacts.sh: xtask runfile is unavailable: $xtask_runfile" >&2
  exit 1
fi

for command in "$@"; do
  args=("$command")
  if [[ "$command" == gen-package-policy-inputs ]]; then
    args+=(--write)
  fi
  printf 'generate: %s\n' "$command"
  (cd "$repo_root" && D2B_REPO_ROOT="$repo_root" "$xtask" "${args[@]}")
done
