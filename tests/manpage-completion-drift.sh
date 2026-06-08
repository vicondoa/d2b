#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

for artifact in \
  docs/manpages/nixling.1 \
  docs/completions/nixling.bash \
  docs/completions/nixling.zsh \
  docs/completions/nixling.fish; do
  [ -f "$artifact" ] || fail "manpage-completion-drift: missing committed artifact $artifact"
done

if [ -z "${NIXLING_MANPAGE_COMPLETION_DRIFT_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "manpage-completion-drift: neither cargo nor nix is on PATH"
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_MANPAGE_COMPLETION_DRIFT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

xtask_bin=$(nl_cargo_bin_path workspace xtask)
if [ ! -x "$xtask_bin" ]; then
  (
    cd packages
    CARGO_TARGET_DIR="$(nl_cargo_target_dir workspace)" cargo build -q --manifest-path "$ROOT/packages/Cargo.toml" -p xtask --bin xtask
  )
fi

log "--> manpage-completion-drift: cargo xtask gen-cli-shell-artifacts"
(
  cd packages
  "$xtask_bin" gen-cli-shell-artifacts
)

if git --no-pager diff --exit-code -- docs/manpages/ docs/completions/ >/dev/null; then
  ok "manpage-completion-drift: generated manpage + completions match committed docs/{manpages,completions}/"
else
  git --no-pager diff -- docs/manpages/ docs/completions/ | head -120 >&2 || true
  fail "manpage-completion-drift: generated manpage + completions drift under docs/{manpages,completions}/"
fi
