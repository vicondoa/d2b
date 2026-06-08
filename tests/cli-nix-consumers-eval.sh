#!/usr/bin/env bash
# tests/cli-nix-consumers-eval.sh—
# regression gate.
#
# Asserts that every consumer of `nixos-modules/cli.nix` has been
# relocated before the sibling deletion removes
# the file. The only acceptable references to cli.nix in the tree
# are:
#
#   * inside `nixos-modules/cli.nix` itself (until the sibling
#     agent deletes it);
#   * inside this gate (the deletion test);
#   * inside the cli-rust-native compat shim (`tests/cli-rust-native-common.sh`,
#     which stages a synthetic `legacy-cli.nix` for byte-parity
#     goldens — not a consumer of the framework's cli.nix);
#   * inside historical / explanatory comments that name cli.nix as
#     a retired surface (allowed; we grep only for live consumer
#     bindings).
#
# Live consumer bindings checked:
#
#   * `config.nixling.cliBin`              — was set by cli.nix,
#                                            consumed by host-audit.nix
#   * `config.nixling.audioStateHelperPath` — was set by cli.nix,
#                                            consumed by tests/audio.sh
#   * `config.nixling._desktopWrappers`    — was set by cli.nix,
#                                            consumed by tests/desktop-wrapper-contract-eval.sh
#   * `config.nixling.store.package`       — was set by store.nix,
#                                            consumed by cli.nix
#   * `config.nixling.store.generations`   — was set by store.nix,
#                                            consumed by cli.nix
#   * `import ./cli.nix` / `imports = [ ... ./cli.nix ... ]`
#                                          — was wired by default.nix
#
# This gate fails closed: any reintroduction of a consumer outside
# the allowlist is a hard FAIL.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/cli-nix-consumers-eval.sh"

cd "$ROOT"

# Helper: collect files that contain a live (non-comment) match for
# the given regex. A "live" match is any line where the regex
# appears before the first `#` character.
live_consumer_hits() {
  local pattern="$1"
  shift
  grep -RIln --exclude-dir=.git --exclude-dir=target \
    --include='*.nix' --include='*.sh' --include='*.rs' \
    -e "$pattern" "$@" . 2>/dev/null \
  | grep -v -E '^\./(nixos-modules/cli\.nix|tests/cli-nix-consumers-eval\.sh)$' \
  | while IFS= read -r f; do
      # Strip everything from the first `#` or `//` onward and re-test.
      # Nix / shell use `#`; rust uses `//`. This is conservative
      # for nix + shell + rust source.
      if sed -e 's/[[:space:]]*#.*$//' -e 's@[[:space:]]*//.*$@@' "$f" \
        | grep -qE "$pattern"; then
        printf '%s\n' "$f"
      fi
    done
}

# 1. config.nixling.cliBin consumers must be gone everywhere except
#    inside cli.nix itself (the emitter).
mapfile -t cliBin_hits < <(live_consumer_hits 'nixling\.cliBin')
if [ "${#cliBin_hits[@]}" -gt 0 ]; then
  printf '  hit: %s\n' "${cliBin_hits[@]}" >&2
  fail "nixling.cliBin still referenced outside cli.nix / this gate"
fi
ok "no live nixling.cliBin consumers outside cli.nix"

# 2. config.nixling.audioStateHelperPath consumers must be gone.
mapfile -t audio_hits < <(live_consumer_hits 'audioStateHelperPath')
if [ "${#audio_hits[@]}" -gt 0 ]; then
  printf '  hit: %s\n' "${audio_hits[@]}" >&2
  fail "audioStateHelperPath still referenced outside cli.nix / this gate"
fi
ok "no live audioStateHelperPath consumers outside cli.nix"

# 3. config.nixling._desktopWrappers consumers must be gone.
mapfile -t wrapper_hits < <(live_consumer_hits '_desktopWrappers')
if [ "${#wrapper_hits[@]}" -gt 0 ]; then
  printf '  hit: %s\n' "${wrapper_hits[@]}" >&2
  fail "_desktopWrappers still referenced outside cli.nix / this gate"
fi
ok "no live _desktopWrappers consumers outside cli.nix"

# 4. nixling.store.package / nixling.store.generations consumers
#    must be gone. cli.nix is the only allowed reader; the
#    declarations have already been removed from store.nix.
mapfile -t store_pkg_hits < <(
  {
    live_consumer_hits 'nixling\.store\.package'
    live_consumer_hits 'nixling\.store\.generations'
  } | sort -u \
    | grep -v -E '^\./tests/static\.sh$' \
    || true
)
if [ "${#store_pkg_hits[@]}" -gt 0 ]; then
  printf '  hit: %s\n' "${store_pkg_hits[@]}" >&2
  fail "nixling.store.package/generations referenced outside cli.nix + static.sh trio lint"
fi
ok "no live nixling.store.{package,generations} consumers outside cli.nix"

# 5. No module imports ./cli.nix any more.
mapfile -t import_hits < <(
  grep -RIln --exclude-dir=.git --exclude-dir=target \
    --include='*.nix' -e './cli\.nix' . 2>/dev/null \
  | grep -v -E '^\./(nixos-modules/cli\.nix|tests/cli-nix-consumers-eval\.sh)$' \
  || true
)
# Permit comment-only references; check that the matching line is not
# an `import` / `imports = [` payload.
for path in "${import_hits[@]:-}"; do
  [ -n "$path" ] || continue
  if grep -nE '^[^#]*(\bimport\b|imports[[:space:]]*=)[^#]*\./cli\.nix' "$path" >/dev/null; then
    grep -nE '\./cli\.nix' "$path" >&2 || true
    fail "$path still imports ./cli.nix"
  fi
done
ok "no live import ./cli.nix outside cli.nix / this gate"

# 6. Sanity: cli.nix still exists on disk in this commit (the
#    sibling agent owns the actual deletion).
#    If the file is gone, the gate trivially passes — that's the
#    desired post-sibling-merge end state.
if [ -f nixos-modules/cli.nix ]; then
  ok "nixos-modules/cli.nix still on disk (sibling agent owns deletion)"
else
  ok "nixos-modules/cli.nix already deleted (sibling agent merged)"
fi

log "OK: cli.nix consumer surface emptied"
