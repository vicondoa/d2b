#!/usr/bin/env bash
# tests/tracing-contract-lint.sh—
#
# Static enforcement of the bounded-cardinality tracing-attribute
# allowlist documented in docs/reference/tracing-contract.md.
#
# Greps workspace Rust source (packages/**/*.rs, excluding generated
# / vendored trees) for `tracing!`-style call sites and fails closed
# if any of the historically-forbidden high-cardinality / leakable
# attribute shapes appear.
#
# Forbidden patterns enforced here track previously fixed regressions:
#   - bundle=%path / path=%path were removed from the broker
#     bundle-load span.
#   - path=%keys_dir.display() was removed from the
#     ssh-host-key-preflight tracing site.
#   - drift_kind is bounded by a typed enum attribute.
#   - the residual path=%path.display() debug! call was removed.
#
# Allowlist tail (cited verbatim from the contract doc):
#   vm, env, role, step_id, operation, outcome, error_kind, op_count,
#   elapsed_ms, parent_pid, exit, load_outcome, reason, drift_kind,
#   plus bounded numeric intent counts (nft, route, sysctl, tap,
#   bridge, ...).
#
# Per-VM bounded path attributes (e.g. `path = %spec.path` for the
# canonical ownership matrix at /var/lib/nixling/state/<vm>/<leaf>)
# are tolerated; this gate refuses only the bundle / store-path /
# argv / secret / child-output classes.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# Use the shared lib helpers if available; otherwise fall back to
# inline log/ok/fail so the gate can be run standalone in CI sandboxes
# that bring up only this script.
if [ -f "$HERE/lib.sh" ]; then
  # shellcheck source=lib.sh
  . "$HERE/lib.sh"
else
  log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
  ok()   { log "  PASS: $*"; }
  fail() { log "  FAIL: $*"; exit 1; }
fi

log "==> tests/tracing-contract-lint.sh"

cd "$ROOT"

# Collect tracked Rust source under packages/. We deliberately exclude:
#   - target/  (build artefacts)
#   - tests/   (test fixtures may stub attrs)
#   - vendor/  (3rd-party)
#   - generated/ trees
# but we intentionally INCLUDE packages/**/src/**/*.rs and
# packages/**/tests/**/*.rs because integration tests can regress
# the contract too.
mapfile -t rust_files < <(
  find packages -type f -name '*.rs' \
    -not -path '*/target/*' \
    -not -path '*/vendor/*' \
    -not -path '*/generated/*' \
    2>/dev/null | sort
)

if [ "${#rust_files[@]}" -eq 0 ]; then
  fail "no Rust source files found under packages/ — wrong CWD?"
fi

log "  scanning ${#rust_files[@]} Rust source files"

# A grep helper that returns the matching `path:line:content` lines
# but never causes the script to exit on a non-match (set -e).
scan() {
  local description="$1"
  local pattern="$2"
  local hits
  if hits=$(grep -nE "$pattern" "${rust_files[@]}" 2>/dev/null); then
    log "  VIOLATION: ${description}"
    printf '%s\n' "$hits" | sed 's/^/    /' >&2
    return 1
  fi
  return 0
}

violations=0

# -- 1. Bundle path identifiers -----------------------------
# `bundle = %X.display()`, `bundle = ?X.display()`, `bundle = %X` where
# X plausibly carries a Path/PathBuf — we match any `bundle = %`/`?`
# attribute with `.display()` or `_path` / `bundle_path` aliases.
scan "bundle = %X.display() (P0fu3 H2 — forbidden high-cardinality store path)" \
  'bundle[[:space:]]*=[[:space:]]*[%?][^,]*\.display\(\)' \
  || violations=$((violations + 1))

scan "bundle_path = %X or bundle_path = ?X (alias of forbidden bundle attr)" \
  'bundle_path[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

# -- 2. ssh-host-key keys_dir leak -------------------------
scan "keys_dir = %X.display() (P1fu1 — surface via outcome + audit instead)" \
  'keys_dir[[:space:]]*=[[:space:]]*[%?][^,]*\.display\(\)' \
  || violations=$((violations + 1))

# -- 3. /nix/store literal strings inside tracing arg lists ------------
# We look for any /nix/store/... literal in source. Outside of test
# fixtures (excluded above) the only legitimate places are top-of-file
# module docs / comments; production tracing call sites must never pin
# the store hash into the trace backend.
nix_store_hits=$(
  grep -nE '"/nix/store/[^"]+"' "${rust_files[@]}" 2>/dev/null \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
    || true
)
if [ -n "$nix_store_hits" ]; then
  # Only fail if the literal sits inside a tracing-macro arg list.
  # Heuristic: the same file has the literal AND a tracing! call within
  # the prior ~5 lines. Cheaper, deterministic: scan via awk.
  bad=$(
    awk '
      /tracing::(info|warn|error|debug|trace|event|span)!|^[[:space:]]*(info|warn|error|debug|trace)!\(/ {
        in_tr = 1; depth = 0;
      }
      in_tr {
        for (i = 1; i <= length($0); i++) {
          c = substr($0, i, 1);
          if (c == "(") depth++;
          else if (c == ")") { depth--; if (depth <= 0) { in_tr = 0; break } }
        }
        if (match($0, /"\/nix\/store\/[^"]+"/)) {
          print FILENAME ":" FNR ":" $0;
        }
      }
    ' "${rust_files[@]}" 2>/dev/null || true
  )
  if [ -n "$bad" ]; then
    log "  VIOLATION: /nix/store literal inside a tracing macro arg list (P0fu3 H2)"
    printf '%s\n' "$bad" | sed 's/^/    /' >&2
    violations=$((violations + 1))
  fi
fi

# -- 4. Argv / command-line content --------------------
scan "argv = ... in tracing (forbidden — operator-supplied content; route via typed envelope)" \
  '(^|[^_a-zA-Z])argv[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "cmdline = ... in tracing (forbidden — see argv rule)" \
  '(^|[^_a-zA-Z])cmdline[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "command_line = ... in tracing (forbidden — see argv rule)" \
  '(^|[^_a-zA-Z])command_line[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

# -- 5. Secrets / credential leaks ---------------------
scan "secret = ... in tracing (forbidden — credential leak)" \
  '(^|[^_a-zA-Z])secret[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "password = ... in tracing (forbidden — credential leak)" \
  '(^|[^_a-zA-Z])password[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "token = ... in tracing (forbidden — credential leak)" \
  '(^|[^_a-zA-Z])token[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "private_key = ... in tracing (forbidden — credential leak)" \
  'private_key[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

# -- 6. Child-process output bytes ---------------------
# stdout / stderr as %X attrs would dump child output into the trace
# backend; the typed-error envelope is the right channel for that.
scan "stdout = %X in tracing (forbidden — child output; route via typed envelope)" \
  '(^|[^_a-zA-Z])stdout[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

scan "stderr = %X in tracing (forbidden — child output; route via typed envelope)" \
  '(^|[^_a-zA-Z])stderr[[:space:]]*=[[:space:]]*[%?]' \
  || violations=$((violations + 1))

if [ "$violations" -gt 0 ]; then
  fail "$violations tracing-contract violation class(es) detected — see docs/reference/tracing-contract.md"
fi

ok "no forbidden high-cardinality / leakable tracing attrs detected"
log "==> tracing-contract-lint OK"
