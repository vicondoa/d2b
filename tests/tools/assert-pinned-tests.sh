#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
DEFAULT_PINNED_DIR="$ROOT/tests/golden/pinned"

if ! command -v cargo >/dev/null 2>&1; then
  # Read the channel rather than hardcoding it: a stale literal here silently
  # stops matching after a pin bump, and the script then falls through to
  # whatever cargo the surrounding shell provides - asserting the pinned test
  # inventory under a compiler that is not the pinned one.
  pinned_channel=$(
    sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' \
      "$ROOT/packages/rust-toolchain.toml" | head -1
  )
  for candidate in "$HOME"/.rustup/toolchains/"${pinned_channel:-0.0.0}"-*/bin; do
    if [ -x "$candidate/cargo" ]; then
      PATH="$candidate:$PATH"
      export PATH
      break
    fi
  done
fi

if ! cargo nextest --version >/dev/null 2>&1; then
  if [ -z "${D2B_ASSERT_PINNED_IN_NIX_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    export D2B_ASSERT_PINNED_IN_NIX_SHELL=1
    exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-nextest nixpkgs#gcc \
      --command bash "$0" "$@"
  fi
  echo "assert-pinned-tests: cargo-nextest is required" >&2
  exit 1
fi

# No RUSTC_WRAPPER override here. The cargo configs route through
# .cargo/rustc-wrapper.sh, which uses sccache when present and plain rustc when
# not, so this gate no longer has to disable the compiler cache to stay robust
# in a shell that omits nixpkgs#sccache.

pinned_inputs=("$@")
if [ "${#pinned_inputs[@]}" -eq 0 ]; then
  pinned_inputs=("$DEFAULT_PINNED_DIR")
fi

pinned_files=()
for input in "${pinned_inputs[@]}"; do
  if [ -d "$input" ]; then
    shopt -s nullglob
    dir_files=("$input"/*.txt)
    shopt -u nullglob
    pinned_files+=("${dir_files[@]}")
  elif [ -f "$input" ]; then
    pinned_files+=("$input")
  else
    echo "assert-pinned-tests: missing pinned test list: $input" >&2
    exit 1
  fi
done

if [ "${#pinned_files[@]}" -eq 0 ]; then
  echo "assert-pinned-tests: no pinned test list files found" >&2
  exit 1
fi

declare -A present
collect_present() {
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    present["$line"]=1
    present["${line#* }"]=1
  done
}

# `nextest list --locked` is required to be non-mutating. Preserve all three
# candidate views around both listings so a Cargo regression cannot silently
# rewrite the candidate while asserting its inventory.
candidate_tracked_before=$(git -C "$ROOT" diff --no-ext-diff)
candidate_staged_before=$(git -C "$ROOT" diff --cached --no-ext-diff)
candidate_untracked_before=$(git -C "$ROOT" status --porcelain=v1 --untracked-files=all)

collect_root_listing() {
  local listing
  if ! listing=$(
    cd "$ROOT/packages"
    cargo nextest list --locked --workspace --message-format oneline
  ); then
    echo "assert-pinned-tests: root-workspace nextest listing failed" >&2
    exit 1
  fi
  collect_present <<<"$listing"
}

# The product workspace is the only product Cargo lock authority. The generic
# listing covers every root-workspace package, including the guest package.
collect_root_listing

collect_broker_listing() {
  local listing
  if ! listing=$(
    cd "$ROOT/packages"
    cargo nextest list --locked -p d2b-priv-broker --no-default-features --features layer1-bootstrap,fake-backends --message-format oneline
  ); then
    echo "assert-pinned-tests: broker nextest listing failed" >&2
    exit 1
  fi
  collect_present <<<"$listing"
}

# The broker feature listing is selected from that same root workspace and
# deliberately covers the union of the three serial broker contexts.
collect_broker_listing
candidate_tracked_after=$(git -C "$ROOT" diff --no-ext-diff)
candidate_staged_after=$(git -C "$ROOT" diff --cached --no-ext-diff)
candidate_untracked_after=$(git -C "$ROOT" status --porcelain=v1 --untracked-files=all)
if [ "$candidate_tracked_before" != "$candidate_tracked_after" ] \
  || [ "$candidate_staged_before" != "$candidate_staged_after" ] \
  || [ "$candidate_untracked_before" != "$candidate_untracked_after" ]; then
  echo "assert-pinned-tests: Cargo nextest inventory changed tracked, staged, or untracked candidate state" >&2
  exit 1
fi

declare -A seen
total=0
missing=0
duplicates=0
for pinned_file in "${pinned_files[@]}"; do
  while IFS= read -r pinned || [ -n "$pinned" ]; do
    case "$pinned" in
      ""|\#*) continue ;;
    esac
    total=$((total + 1))
    if [ "${seen[$pinned]+set}" = set ]; then
      echo "assert-pinned-tests: duplicate pinned test: $pinned ($pinned_file)" >&2
      duplicates=$((duplicates + 1))
      continue
    fi
    seen["$pinned"]=1
    if [ "${present[$pinned]+set}" != set ]; then
      echo "assert-pinned-tests: missing pinned test: $pinned ($pinned_file)" >&2
      missing=$((missing + 1))
    fi
  done < "$pinned_file"
done

if [ "$total" -eq 0 ]; then
  echo "assert-pinned-tests: no pinned tests found in ${pinned_files[*]}" >&2
  exit 1
fi

if [ "$missing" -ne 0 ] || [ "$duplicates" -ne 0 ]; then
  echo "assert-pinned-tests: failed ($missing missing, $duplicates duplicate, $total pinned)" >&2
  exit 1
fi

echo "assert-pinned-tests: all $total pinned tests present (${#pinned_files[@]} file(s))"
