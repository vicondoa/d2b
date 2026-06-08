#!/usr/bin/env bash
# v1.1 / ADR 0017: enforce "Rust CLI never executes bash".
#
# Three modes:
#   - check            (Layer 1): ripgrep for Command::new("bash"|"sh"|...)
#                       hits across packages/*/src/; allow-list any
#                       legitimate uses via tests/fixtures/no-bash-exec-exempt-paths.json.
#   - fixture-coverage (Layer 2): assert every entry in the allow-list
#                       fixture still exists as a source line; no
#                       stale entries.
#   - syn-ast-walk     (Layer 3): hand-off to tests/tools/no-bash-ast-walker/
#                       (deferred to v1.1 implementation cycle; currently
#                       a stub that prints a clear "deferred" message
#                       and exits 0 — the syn AST walker requires the
#                       cargoExpandShell devShell per ADR 0017 which
#                       is part of the same TDD row; both land together
#                       in the same closure commit. The check + fixture-
#                       coverage modes provide regression coverage in
#                       the interim).
#
# Usage:
#   tests/no-bash-exec-eval.sh check
#   tests/no-bash-exec-eval.sh fixture-coverage
#   tests/no-bash-exec-eval.sh syn-ast-walk
#   tests/no-bash-exec-eval.sh all
#
# Exit codes:
#   0 — invariant holds
#   1 — invariant violated
#   2 — usage / configuration error
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
EXEMPT_PATHS_FIXTURE="$HERE/fixtures/no-bash-exec-exempt-paths.json"

usage() {
  cat >&2 <<'USAGE'
usage: no-bash-exec-eval.sh {check|fixture-coverage|syn-ast-walk|all}
USAGE
  exit 2
}

mode=${1:-}
if [ -z "$mode" ]; then
  usage
fi

# Bash exec regex covers:
#   Command::new("bash")
#   Command::new("/bin/sh")
#   Command::new("/bin/bash")
#   Command::new("/usr/bin/env"), with bash/sh follow-up arg
#   std::process::Command::new(...) with any of the above
BASH_EXEC_PATTERN='Command::new\("(/bin/|/usr/bin/)?(env(\s+|\s*"\s*,\s*"\s*))?(ba)?sh"'

ensure_exempt_fixture() {
  if [ ! -f "$EXEMPT_PATHS_FIXTURE" ]; then
    printf 'no-bash-exec-eval: missing exempt-paths fixture %s\n' "$EXEMPT_PATHS_FIXTURE" >&2
    exit 2
  fi
}

# Read allow-list (one path per line) from the JSON fixture's
# "exempt_paths" array.
exempt_paths() {
  ensure_exempt_fixture
  # Avoid jq dependency: use python3 if available, else grep heuristic.
  if command -v python3 >/dev/null 2>&1; then
    python3 -c '
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
for p in data.get("exempt_paths", []):
    print(p)
' "$EXEMPT_PATHS_FIXTURE"
  else
    # Fallback: extract every quoted string under "exempt_paths".
    sed -n '/"exempt_paths"/,/]/p' "$EXEMPT_PATHS_FIXTURE" \
      | grep -oP '"\K[^"]+(?=")' \
      | grep -v exempt_paths || true
  fi
}

mode_check() {
  local rg_args=(-n --no-heading -p)
  rg_args+=("$BASH_EXEC_PATTERN")
  rg_args+=("$ROOT/packages")
  rg_args+=(-g '!**/target/**')
  rg_args+=(-g '!**/tests/**')
  rg_args+=(-g '!**/.git/**')

  local hits
  hits=$(rg "${rg_args[@]}" 2>/dev/null || true)
  if [ -z "$hits" ]; then
    printf 'no-bash-exec-eval[check]: PASS (no bash exec sites in packages/*/src)\n'
    return 0
  fi

  # Filter against allow-list.
  local exempt
  exempt=$(exempt_paths)
  local remaining
  remaining=$(printf '%s\n' "$hits" | while read -r line; do
    file=${line%%:*}
    if printf '%s\n' "$exempt" | grep -Fxq -- "${file#$ROOT/}"; then
      continue
    fi
    printf '%s\n' "$line"
  done)

  if [ -z "$remaining" ]; then
    printf 'no-bash-exec-eval[check]: PASS (all hits allow-listed)\n'
    return 0
  fi

  printf 'no-bash-exec-eval[check]: FAIL — found bash exec sites not in allow-list:\n' >&2
  printf '%s\n' "$remaining" >&2
  printf '\nTo allow-list a legitimate use, add the source path (relative\n' >&2
  printf 'to repo root) to tests/fixtures/no-bash-exec-exempt-paths.json.\n' >&2
  printf 'Allow-list additions require panel review per ADR 0017.\n' >&2
  return 1
}

mode_fixture_coverage() {
  ensure_exempt_fixture
  local exempt
  exempt=$(exempt_paths)
  local stale=0
  while IFS= read -r path; do
    [ -z "$path" ] && continue
    if [ ! -e "$ROOT/$path" ]; then
      printf 'no-bash-exec-eval[fixture-coverage]: STALE entry %s (file missing)\n' "$path" >&2
      stale=1
    fi
  done <<< "$exempt"
  if [ "$stale" -ne 0 ]; then
    printf 'no-bash-exec-eval[fixture-coverage]: FAIL — remove stale entries from the fixture\n' >&2
    return 1
  fi
  printf 'no-bash-exec-eval[fixture-coverage]: PASS (no stale entries)\n'
  return 0
}

mode_syn_ast_walk() {
  local walker_dir="$HERE/tools/no-bash-ast-walker"
  if [ -d "$walker_dir" ] && [ -f "$walker_dir/Cargo.toml" ]; then
    # v1.1.1: dedicated AST walker is the deeper-coverage gate
    # (per ADR 0017 § "Toolchain provisioning"). It uses `syn` to
    # visit every `Command::new(...)` ExprCall and refuse any
    # `Command::new("bash"|"sh"|...)` literal.
    ( cd "$walker_dir" && unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER && \
        nix shell nixpkgs#rustc nixpkgs#cargo nixpkgs#gcc --command \
        env CARGO_BUILD_RUSTC_WRAPPER= cargo run --release --quiet -- "$ROOT/packages" )
    return $?
  fi
  printf 'no-bash-exec-eval[syn-ast-walk]: walker tool missing at %s — falling back to check mode\n' "$walker_dir"
  mode_check
}

case "$mode" in
  check)            mode_check ;;
  fixture-coverage) mode_fixture_coverage ;;
  syn-ast-walk)     mode_syn_ast_walk ;;
  all)
    mode_check && mode_fixture_coverage && mode_syn_ast_walk
    ;;
  *) usage ;;
esac
