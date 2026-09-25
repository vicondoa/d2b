#!/usr/bin/env bash
# Refresh the vendored agent-skill trees and the .agents/skills links that
# omp sessions in this repository load, so a fresh clone is configured
# without any install step.
#
# Sources:
#   compound-engineering: github.com/EveryInc/compound-engineering-plugin
#   caveman suite:        github.com/JuliusBrussee/caveman
#   ponytail:             vendored third_party/agent-skills/ponytail (no
#                         upstream marketplace); untouched by this script.
#
# Each refreshed tree carries an UPSTREAM.json provenance manifest
# (repository, commit, version, vendor date, license, per-file sha256 over
# the imported paths) and replaces the previous version directory. Only
# skill directories that contain a SKILL.md are imported; generated or
# hidden entries are skipped. Commit the result after running.

set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

refresh() {
  local repo=$1 name=$2 base=$3
  local work="$tmp/$name"
  git clone --depth 1 --quiet "$repo" "$work"

  local version
  version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$work/package.json" | head -1)
  if [[ -z $version ]]; then
    echo "update-agent-skills: no version field in $name package.json" >&2
    exit 1
  fi

  local dest="$base/v$version"
  rm -rf "$dest"
  mkdir -p "$dest/skills"

  local dir base_name
  while IFS= read -r dir; do
    base_name=$(basename "$dir")
    case "$base_name" in
      generated|.*|"") continue ;;
    esac
    [[ -f "$dir/SKILL.md" ]] || continue
    cp -R "$dir" "$dest/skills/"
  done < <(find "$work/skills" -mindepth 1 -maxdepth 1 -type d | sort)

  if [[ -f $work/LICENSE ]]; then
    cp "$work/LICENSE" "$dest/LICENSE"
  fi

  local commit hashes
  commit=$(git -C "$work" rev-parse HEAD)
  hashes=$(cd "$dest" && find . -type f ! -name UPSTREAM.json | sed 's|^\./||' | sort | \
    while IFS= read -r file; do
      printf '"%s": "%s",\n' "$file" "$(sha256sum "$file" | cut -d' ' -f1)"
    done)
  hashes=${hashes%,}

  {
    printf '{\n'
    printf '  "upstream_repository": "%s",\n' "$repo"
    printf '  "upstream_commit": "%s",\n' "$commit"
    printf '  "upstream_version": "%s",\n' "$version"
    printf '  "vendor_date": "%s",\n' "$(date +%F)"
    printf '  "license": "MIT for the imported skill surfaces; see LICENSE",\n'
    printf '  "excluded_surfaces": ["all other files", "plugin runtime"],\n'
    printf '  "files": {\n%s\n  }\n' "$hashes"
    printf '}\n'
  } > "$dest/UPSTREAM.json"

  # Drop superseded version directories under the same base so exactly one
  # vendored copy remains.
  find "$base" -mindepth 1 -maxdepth 1 -type d ! -name "v$version" -exec rm -rf {} +

  echo "vendored $name $version -> $dest ($(find "$dest" -type f | wc -l) files)"
}

refresh https://github.com/EveryInc/compound-engineering-plugin \
  compound-engineering third_party/agent-skills/compound-engineering
refresh https://github.com/JuliusBrussee/caveman \
  caveman third_party/agent-skills/caveman

# Regenerate the adapter links: one relative symlink per discovered skill
# directory across every vendored tree, in both omp-native (.agents/skills)
# and Claude-compatible (.claude/skills) surfaces. Relative targets keep the
# links portable inside a clone.
rm -rf .agents/skills .claude/skills
mkdir -p .agents/skills .claude/skills

for tree in third_party/agent-skills/compound-engineering/*/skills \
            third_party/agent-skills/caveman/*/skills \
            third_party/agent-skills/ponytail/*/skills; do
  [[ -d $tree ]] || continue
  while IFS= read -r dir; do
    base_name=$(basename "$dir")
    case "$base_name" in
      generated|.*|"") continue ;;
    esac
    [[ -f "$dir/SKILL.md" ]] || continue
    dir=${dir%/}
    ln -sfn "../../$dir" ".agents/skills/$base_name"
    ln -sfn "../../$dir" ".claude/skills/$base_name"
  done < <(find "$tree" -mindepth 1 -maxdepth 1 -type d | sort)
done

echo "agent skills: $(find .agents/skills -mindepth 1 -maxdepth 1 | wc -l) omp links, $(find .claude/skills -mindepth 1 -maxdepth 1 | wc -l) claude links"
