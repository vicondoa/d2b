#!/usr/bin/env bash
# Refresh the vendored agent-skill trees and the .agents/skills links that
# omp sessions in this repository load, so a fresh clone is configured
# without any install step.
#
# Sources:
#   compound-engineering: github.com/EveryInc/compound-engineering-plugin
#   caveman suite:        github.com/JuliusBrussee/caveman
#   rewrite-rs suite:     github.com/rewrite-rs/skills
#   ponytail:             vendored third_party/agent-skills/ponytail (no
#                         upstream marketplace); untouched by this script.
#
# Usage: update-agent-skills.sh [--only <source>]
#   --only refreshes one upstream source and leaves the others as they are, so
#   adding a source does not pick up unrelated upstream drift. Adapter link
#   regeneration always runs over every vendored tree.
#   A source may pass a space-separated exclude list of skills/ subpaths; the
#   manifest records what was skipped.
#
# Each refreshed tree carries an UPSTREAM.json provenance manifest
# (repository, commit, version, vendor date, license, per-file sha256 over
# the imported paths) and replaces the previous version directory. Only
# skill directories that contain a SKILL.md are imported; generated or
# hidden entries are skipped. Commit the result after running.

set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

only=
while [[ $# -gt 0 ]]; do
  case $1 in
    --only)
      only=${2:-}
      if [[ -z $only ]]; then
        echo "usage: $(basename "$0") [--only <source>]" >&2
        exit 2
      fi
      shift 2
      ;;
    *)
      echo "usage: $(basename "$0") [--only <source>]" >&2
      exit 2
      ;;
  esac
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

refresh() {
  local repo=$1 name=$2 base=$3 layout=${4:-flat}
  local license="${5:-MIT for the imported skill surfaces; see LICENSE}"
  local exclude=${6:-}
  if [[ -n $only && $only != "$name" ]]; then
    echo "update-agent-skills: skipped $name (--only $only)"
    return 0
  fi

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

  # flat: skill directories sit directly under skills/. nested: they sit one
  # category level down (skills/<category>/<skill>) and keep that path.
  local -a candidates=()
  if [[ $layout == nested ]]; then
    mapfile -t candidates < <(find "$work/skills" -mindepth 2 -type d | sort)
  else
    mapfile -t candidates < <(find "$work/skills" -mindepth 1 -maxdepth 1 -type d | sort)
  fi

  local dir base_name rel entry
  for dir in ${candidates[@]+"${candidates[@]}"}; do
    base_name=$(basename "$dir")
    case "$base_name" in
      generated|.*|"") continue ;;
    esac
    [[ -f "$dir/SKILL.md" ]] || continue
    rel=${dir#"$work/skills"/}
    for entry in $exclude; do
      case $rel in
        "$entry"|"$entry"/*) continue 2 ;;
      esac
    done
    mkdir -p "$dest/skills/$(dirname "$rel")"
    cp -R "$dir" "$dest/skills/$rel"
  done

  if [[ -f $work/LICENSE ]]; then
    cp "$work/LICENSE" "$dest/LICENSE"
  fi

  local commit hashes excluded_extra=
  for entry in $exclude; do
    excluded_extra+=", \"skills/$entry\""
  done
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
    printf '  "license": "%s",\n' "$license"
    printf '  "excluded_surfaces": ["all other files", "plugin runtime"%s],\n' "$excluded_extra"
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
refresh https://github.com/rewrite-rs/skills \
  rewrite-rs third_party/agent-skills/rewrite-rs nested "BSD-3-Clause; see LICENSE" porting

# Regenerate the adapter links: one relative symlink per discovered skill
# directory across every vendored tree, in both omp-native (.agents/skills)
# and Claude-compatible (.claude/skills) surfaces. Relative targets keep the
# links portable inside a clone.
rm -rf .agents/skills .claude/skills
mkdir -p .agents/skills .claude/skills

for tree in third_party/agent-skills/compound-engineering/*/skills \
            third_party/agent-skills/caveman/*/skills \
            third_party/agent-skills/ponytail/*/skills \
            third_party/agent-skills/rewrite-rs/*/skills; do
  [[ -d $tree ]] || continue
  while IFS= read -r dir; do
    base_name=$(basename "$dir")
    case "$base_name" in
      generated|.*|"") continue ;;
    esac
    [[ -f "$dir/SKILL.md" ]] || continue
    dir=${dir%/}
    for adapter in .agents/skills .claude/skills; do
      if [[ -e $adapter/$base_name ]]; then
        echo "update-agent-skills: duplicate skill name $base_name at $dir" >&2
        exit 1
      fi
      ln -sfn "../../$dir" "$adapter/$base_name"
    done
  done < <(find "$tree" -mindepth 1 -type d | sort)
done

echo "agent skills: $(find .agents/skills -mindepth 1 -maxdepth 1 | wc -l) omp links, $(find .claude/skills -mindepth 1 -maxdepth 1 | wc -l) claude links"
