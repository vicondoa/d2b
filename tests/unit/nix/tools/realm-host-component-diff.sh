#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --candidate-root <component-worktree>" >&2
  exit 2
}

candidate_arg=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --candidate-root)
      [ "$#" -ge 2 ] || usage
      candidate_arg=$2
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done
[ -n "$candidate_arg" ] || usage

for command in git jq nix-instantiate; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "component ownership: missing required command: $command" >&2
    exit 1
  }
done
export GIT_NO_REPLACE_OBJECTS=1

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
trusted_root=$(cd "$here/../../../.." && pwd -P)
candidate_root=$(cd "$candidate_arg" && pwd -P)
policy="$trusted_root/tests/unit/nix/eval-cases/realm-host-component-policy.nix"

git -C "$trusted_root" diff --quiet --ignore-submodules=none
git -C "$trusted_root" diff --cached --quiet --ignore-submodules=none
[ -z "$(git -C "$trusted_root" status --porcelain=v1 --untracked-files=all)" ] || {
  echo "component ownership: trusted W7 worktree is dirty" >&2
  exit 1
}
[ "$(git -C "$trusted_root" branch --show-current)" = "adr0045-w7-realm-host" ] || {
  echo "component ownership: trusted worktree is not adr0045-w7-realm-host" >&2
  exit 1
}

git -C "$candidate_root" diff --quiet --ignore-submodules=none
git -C "$candidate_root" diff --cached --quiet --ignore-submodules=none
[ -z "$(git -C "$candidate_root" status --porcelain=v1 --untracked-files=all)" ] || {
  echo "component ownership: candidate worktree is dirty" >&2
  exit 1
}

common_dir() {
  local root=$1
  local common
  common=$(git -C "$root" rev-parse --git-common-dir)
  if [[ "$common" = /* ]]; then
    cd "$common" && pwd -P
  else
    cd "$root/$common" && pwd -P
  fi
}

trusted_common=$(common_dir "$trusted_root")
candidate_common=$(common_dir "$candidate_root")
[ "$trusted_common" = "$candidate_common" ] || {
  echo "component ownership: candidate is not a worktree of the trusted repository" >&2
  exit 1
}
[ ! -e "$trusted_common/info/grafts" ] || {
  echo "component ownership: graft metadata is forbidden" >&2
  exit 1
}
[ ! -e "$trusted_common/shallow" ] || {
  echo "component ownership: shallow metadata is forbidden" >&2
  exit 1
}
[ -z "$(git -C "$trusted_root" for-each-ref --format='%(refname)' refs/replace)" ] || {
  echo "component ownership: replacement refs are forbidden" >&2
  exit 1
}

base=$(git -C "$trusted_root" rev-parse HEAD)
head=$(git -C "$candidate_root" rev-parse HEAD)
[ "$base" != "$head" ] || {
  echo "component ownership: candidate has no committed change" >&2
  exit 1
}
git -C "$candidate_root" -c core.useReplaceRefs=false \
  merge-base --is-ancestor "$base" "$head" || {
  echo "component ownership: candidate does not descend from trusted W7 HEAD" >&2
  exit 1
}

branch=$(git -C "$candidate_root" branch --show-current)
[[ "$branch" =~ ^adr0045-w7-[a-z0-9-]+$ ]] || {
  echo "component ownership: candidate branch is not canonical" >&2
  exit 1
}

paths_json=$(
  git -C "$candidate_root" \
    -c core.useReplaceRefs=false \
    -c diff.ignoreSubmodules=none \
    diff --name-only -z --no-renames --ignore-submodules=none "$base" "$head" |
    jq -Rs 'split("\u0000") | map(select(length > 0))'
)
result=$(
  nix-instantiate --eval --strict --json "$policy" \
    --argstr branch "$branch" \
    --argstr pathsJson "$paths_json"
)

if [ "$(jq -r '.valid' <<<"$result")" != "true" ]; then
  jq -r '
    "component ownership: denied"
    + "\n  component: " + (.component // "<unrecognized>")
    + "\n  violations: " + (.violations | join(", "))
    + "\n  blocked dependencies: "
    + (.blockedExternalDependencies | join(", "))
  ' <<<"$result" >&2
  exit 1
fi

jq -r '
  "component ownership: ok ("
  + .component
  + ", "
  + ((.paths | length) | tostring)
  + " changed path(s))"
' <<<"$result"
