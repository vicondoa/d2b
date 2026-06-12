#!/usr/bin/env bash
# Layer-1 static checks for the nixling framework.
#
# Runs in seconds; catches:
#   - syntax errors in any nixling .nix file
#   - missing imports / option-type mismatches (via dry-build)
#   - `flake check` failures (eval of every package output)
#   - per-VM closure attributes failing to evaluate
#
# Exits non-zero on the first failure. Safe to run on any commit.
#
# Usage:
#   tests/static.sh

set -euo pipefail

# Derive ROOT from the script's own location (one dir above tests/) so
# `tests/static.sh` works from any clone of the repo, not just the
# maintainer's /etc/nixos checkout. Override with ROOT=/path/to/clone.
HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
export ROOT
export FLAKE=${FLAKE:-$ROOT}
# Set a placeholder before sourcing lib.sh so it does not eagerly create
# a fallback smoke cache that would then be mistaken for an orphan.
export NL_STATIC_CACHE="$ROOT/.static-cache.bootstrap"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

# Coarse-grained inter-process serializer. Concurrent invocations of
# `tests/static.sh` against the same worktree (e.g. the integrator
# running it while a review-fleet sub-agent also runs it on the same
# checkout) compete for the nix-daemon socket and store-path locks.
# Under load that surfaces as transient "could not render smoke
# vms.json" failures inside the Layer-1 flake-eval gates. Serialize
# concurrent runs per worktree on a single flock so the daemon only
# sees one Layer-1 evaluator at a time.
#
# Implementation note (flock-fd fix): we use `flock(1)` as an exec
# wrapper around a re-entry of this script ($0 --internal-locked). That
# way the lock fd is OWNED BY THE flock(1) PROCESS, not by the inner
# bash. Children spawned inside the gate (broker test daemons, etc.) do
# NOT inherit the lock fd via fork+exec, so a leaked-child daemon can't
# keep the lock alive past its parent's exit. The prior in-shell
# `exec {fd}>file; flock -x $fd` pattern leaked fd 10 into every child
# and caused multi-minute deadlocks when test broker daemons outlived
# their spawning shell.
#
# Bypass the lock with NL_STATIC_NO_LOCK=1 when known-safe (e.g. CI
# already isolates the worktree).
if [ -z "${NL_STATIC_NO_LOCK:-}" ] \
   && [ "${1:-}" != "--internal-locked" ] \
   && command -v flock >/dev/null 2>&1; then
  _STATIC_LOCK="$ROOT/.static-sh.lock"
  : > "$_STATIC_LOCK"
  # Leaked-sccache safety net. Cargo's `sccache` server
  # daemonises off the bash that ran cargo and inherits the gate's
  # flock fd (fd 3). If a previous gate run didn't run `sccache
  # --stop-server` at exit, that sccache is still alive holding fd
  # 3 → our new flock blocks indefinitely on a phantom holder. Scan
  # for any sccache that has the lock file open and kill it before
  # we even try to acquire. The post-gate cleanup at the bottom
  # of static.sh now also runs `sccache --stop-server` so this
  # safety net is normally a no-op; it's here for the recovery
  # path when an older static.sh exited without stopping sccache.
  if command -v sccache >/dev/null 2>&1; then
    _sccache_inode=$(stat -c '%i' "$_STATIC_LOCK" 2>/dev/null || true)
    if [ -n "$_sccache_inode" ]; then
      for _candidate_pid in /proc/[0-9]*/comm; do
        [ -e "$_candidate_pid" ] || continue
        case "$(cat "$_candidate_pid" 2>/dev/null)" in
          sccache) ;;
          *) continue ;;
        esac
        _pid=${_candidate_pid#/proc/}
        _pid=${_pid%/comm}
        for _fd in /proc/"$_pid"/fd/*; do
          [ -e "$_fd" ] || continue
          _target=$(readlink -f "$_fd" 2>/dev/null) || continue
          if [ "$_target" = "$_STATIC_LOCK" ]; then
            printf '%s reap leaked sccache PID %s holding %s\n' \
              "$(date +%H:%M:%S)" "$_pid" "$_STATIC_LOCK" >&2
            kill "$_pid" 2>/dev/null || true
            break
          fi
        done
      done
      unset _sccache_inode _candidate_pid _pid _fd _target
    fi
  fi
  exec flock -x "$_STATIC_LOCK" "$0" --internal-locked "$@"
fi
# When we reach here we are either NL_STATIC_NO_LOCK=1 or we are the
# inner locked re-entry. In the latter case, strip the sentinel arg.
if [ "${1:-}" = "--internal-locked" ]; then
  shift
fi

reap_known_static_orphans() {
  local pattern candidate
  local -a patterns=(
    '.agent-tmp'
    '.audio-gitcfg.*'
    '.assertions-eval.*'
    '.broad-caps-invariant.*'
    '.broker-*'
    '.bundle-drift.*'
    '.cli-contract-coverage.*'
    '.cli-json.*'
    '.cli-json-drift.*'
    '.cli-legacy-bash-dispatch.*'
    '.cli-rust-native-audit.*'
    '.cli-rust-native-auth-status.*'
    '.cli-rust-native-cache'
    '.cli-rust-native-host-check.*'
    '.cli-rust-native-list.*'
    '.cli-rust-native.log'
    '.cli-rust-native-status.*'
    '.cli-rust-native-usb.*'
    '.daemon-*'
    '.host-check.*'
    '.nixlingd-startup-smoke.*'
    '.manifest-fuzz-bounded.*'
    '.manifest-gate.*'
    '.manifest-v04-roundtrip.*'
    '.nl-smoke-cache.*'
    '.nixling-rust-gate.*'
    '.nixling-stub-smoke.*'
    '.nixling-test.log'
    '.observability-eval.*'
    '.opaque-key-ids.*'
    '.privileges-matrix.*'
    '.runner-shape-snapshot.log'
    '.static-cache.*'
    '.static-gitcfg.*'
    '.template-flake-check.*'
    '.uid0-invariant.*'
    '.vms-json-parity.*'
    '.world-readable-invariant.*'
    '.writable-paths-invariant.*'
  )
  shopt -s nullglob
  # Skip the current bash's own bookkeeping (NL_CLEANUPS_FILE = .nl-cleanups.<PID>)
  # and the immediate parent's (flock-wrap chain). Earlier behaviour reaped both
  # because the pattern `.nl-cleanups.*` matched globally; that emptied this
  # run's cleanups file mid-run and meant subsequent `add_cleanup` calls only
  # showed up in the freshly-recreated file, dropping any cleanups registered
  # between lib.sh source and the reaper. Observed in the run where the
  # current re-entry's own .nl-cleanups.<bashpid> was reaped at startup.
  local _self_cleanups_file="${NL_CLEANUPS_FILE:-}"
  local _self_cleanups_basename=""
  [ -n "$_self_cleanups_file" ] && _self_cleanups_basename=$(basename "$_self_cleanups_file")
  for pattern in "${patterns[@]}"; do
    for candidate in "$ROOT"/$pattern; do
      [ -e "$candidate" ] || continue
      [ "$candidate" = "$ROOT/.static-sh.lock" ] && continue
      if [ -n "$_self_cleanups_basename" ] \
         && [ "$(basename "$candidate")" = "$_self_cleanups_basename" ]; then
        continue
      fi
      log "reap known orphan: $candidate"
      rm -rf -- "$candidate"
    done
  done
  shopt -u nullglob
}

NL_STATIC_JOBS=${NL_STATIC_JOBS:-4}
export NL_STATIC_JOBS
NL_STATIC_PARALLEL_ACTIVE=0

declare -A NL_STATIC_PARALLEL_LABEL=()
declare -A NL_STATIC_PARALLEL_LOG=()
declare -A NL_STATIC_PARALLEL_STATUS=()
declare -A NL_STATIC_PARALLEL_DONE=()

nl_static_gate_begin() {
  local label="$1" message="$2"
  nl_time_begin "$label"
  log "--> $message"
}

nl_static_gate_end() {
  nl_time_end "$1"
}

nl_static_path_prefix() {
  local shell_path="$1" base_path="$2"
  case "$shell_path" in
    "$base_path") printf '%s\n' "" ;;
    *":$base_path") printf '%s\n' "${shell_path%:$base_path}" ;;
    *) printf '%s\n' "$shell_path" ;;
  esac
}

nl_static_parallel_key() {
  printf '%s\n' "$1" | tr -cs 'A-Za-z0-9._-' '-'
}

# Parallel-test timing artifacts MUST live outside $ROOT so they cannot
# race with `builtins.getFlake (toString $ROOT)` source captures. Nix
# enumerates and copies the entire flake source tree at first eval; if
# a parallel test's status/log file is created or removed between the
# stat and the copy, the copy fails with
#   error: path '//<flake source>/.static-timing.status.<name>' does
#   not exist
# (observed in cli-legacy-bash-dispatch / cli-json timing runs).
# Putting these under
# ${TMPDIR:-/tmp}/nixling-static-timing.$$/ keeps them per-static.sh
# run, cleaned up by run_cleanups on EXIT, and invisible to flake
# source capture.
_NL_STATIC_TIMING_DIR="${TMPDIR:-/tmp}/nixling-static-timing.$$"
mkdir -p "$_NL_STATIC_TIMING_DIR"
add_cleanup "rm -rf -- $(printf '%q' "$_NL_STATIC_TIMING_DIR")"

nl_static_parallel_log_path() {
  local key
  key=$(nl_static_parallel_key "$1")
  printf '%s\n' "$_NL_STATIC_TIMING_DIR/log.$key"
}

nl_static_parallel_status_path() {
  local key
  key=$(nl_static_parallel_key "$1")
  printf '%s\n' "$_NL_STATIC_TIMING_DIR/status.$key"
}

nl_static_parallel_abort() {
  local pid
  for pid in "${!NL_STATIC_PARALLEL_LABEL[@]}"; do
    [ -n "${NL_STATIC_PARALLEL_DONE[$pid]:-}" ] && continue
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" >/dev/null 2>&1 || true
    fi
  done
  for pid in "${!NL_STATIC_PARALLEL_LABEL[@]}"; do
    [ -n "${NL_STATIC_PARALLEL_DONE[$pid]:-}" ] && continue
    wait "$pid" >/dev/null 2>&1 || true
  done
}

nl_static_parallel_harvest() {
  local pid rc label log_path status_path
  for pid in "${!NL_STATIC_PARALLEL_LABEL[@]}"; do
    [ -n "${NL_STATIC_PARALLEL_DONE[$pid]:-}" ] && continue
    status_path=${NL_STATIC_PARALLEL_STATUS[$pid]}
    [ -f "$status_path" ] || continue
    rc=$(cat "$status_path")
    rm -f -- "$status_path"
    NL_STATIC_PARALLEL_DONE[$pid]=1
    NL_STATIC_PARALLEL_ACTIVE=$((NL_STATIC_PARALLEL_ACTIVE - 1))
    label=${NL_STATIC_PARALLEL_LABEL[$pid]}
    log_path=${NL_STATIC_PARALLEL_LOG[$pid]}
    if [ "$rc" -ne 0 ]; then
      [ -f "$log_path" ] && cat "$log_path" >&2 || true
      nl_static_parallel_abort
      fail "$label"
    fi
    ok "$label"
  done
}

nl_static_parallel_wait_one() {
  wait -n || true
  nl_static_parallel_harvest
}

nl_static_parallel_wait_all() {
  while [ "$NL_STATIC_PARALLEL_ACTIVE" -gt 0 ]; do
    nl_static_parallel_wait_one
  done
}

nl_static_parallel_spawn() {
  local timer_begun=0 label log_path status_path pid
  if [ "${1:-}" = "--timer-begun" ]; then
    timer_begun=1
    shift
  fi
  label="$1"
  shift
  while [ "$NL_STATIC_PARALLEL_ACTIVE" -ge "$NL_STATIC_JOBS" ]; do
    nl_static_parallel_wait_one
  done
  log_path=$(nl_static_parallel_log_path "$label")
  status_path=$(nl_static_parallel_status_path "$label")
  rm -f -- "$log_path" "$status_path"
  if [ "$timer_begun" -eq 0 ]; then
    nl_time_begin "$label"
  fi
  (
    local rc=0
    set +e
    "$@" >"$log_path" 2>&1
    rc=$?
    set -e
    nl_time_end "$label"
    printf '%s\n' "$rc" > "$status_path"
    exit 0
  ) &
  pid=$!
  NL_STATIC_PARALLEL_LABEL[$pid]="$label"
  NL_STATIC_PARALLEL_LOG[$pid]="$log_path"
  NL_STATIC_PARALLEL_STATUS[$pid]="$status_path"
  NL_STATIC_PARALLEL_ACTIVE=$((NL_STATIC_PARALLEL_ACTIVE + 1))
}

nl_static_parallel_script() {
  local label="$1" path="$2"
  nl_static_parallel_spawn "$label" bash "$path"
}

nl_static_parallel_script_gate() {
  local label="$1" path="$2"
  nl_static_gate_begin "$label" "$label"
  nl_static_parallel_spawn --timer-begun "$label" bash "$path"
}

nl_static_run_smoke_eval() {
  local path="$1" expr="$2" ok_label="$3" tail_lines="${4:-20}"
  [ -f "$path" ] || return 0
  if nix-instantiate --eval --strict --expr "$expr" >/dev/null 2>&1; then
    ok "$ok_label"
  else
    nix-instantiate --eval --strict --expr "$expr" 2>&1 | tail -n "$tail_lines" >&2 || true
    fail "$ok_label"
  fi
}

nl_static_parallel_smoke_eval_gate() {
  local label="$1" path="$2" expr="$3" ok_label="$4" tail_lines="${5:-20}"
  nl_static_gate_begin "$label" "$label"
  nl_static_parallel_spawn --timer-begun "$label" nl_static_run_smoke_eval "$path" "$expr" "$ok_label" "$tail_lines"
}

nl_reap_scratch_orphans
reap_known_static_orphans

# Preflight: fail-closed before any disk-consuming setup (rust toolchain
# bootstrap below pulls multi-GiB into /nix/store via nix shell). Runs
# AFTER the orphan reapers above so a recoverable-by-reap situation
# isn't flagged spuriously, but BEFORE any nix-store realisation work.
nl_static_gate_begin "tests/preflight-disk-space.sh" "tests/preflight-disk-space.sh"
bash "$ROOT/tests/preflight-disk-space.sh"
ok "preflight-disk-space"
nl_static_gate_end "tests/preflight-disk-space.sh"

if [ -d "$ROOT/packages" ] && [ -f "$ROOT/packages/rust-toolchain.toml" ]; then
  nl_time_begin "shared rust toolchain bootstrap"
  _STATIC_RUST_SHELL_PATH=$(nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#rustup nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy \
    nixpkgs#gcc nixpkgs#sccache nixpkgs#cargo-deny nixpkgs#cargo-audit \
    --command bash -lc 'printf %s "$PATH"')
  NL_RUST_TOOLCHAIN_PATH=$(nl_static_path_prefix "$_STATIC_RUST_SHELL_PATH" "$PATH")
  export NL_RUST_TOOLCHAIN_PATH
  _STATIC_PINNED_RUST_CHANNEL=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$ROOT/packages/rust-toolchain.toml" | head -1)
  if [ -n "$_STATIC_PINNED_RUST_CHANNEL" ]; then
    export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$_STATIC_PINNED_RUST_CHANNEL}"
    nl_activate_rust_toolchain_path || true
    if command -v rustup >/dev/null 2>&1; then
      rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal --component rustfmt --component clippy >/dev/null 2>&1 || true
    fi
  fi
  nl_time_end "shared rust toolchain bootstrap"
fi

# Scope a safe.directory entry for $ROOT to libgit2 (used by
# `nix flake check` and `nix eval`) without mutating the caller's git
# config. Pattern is the same as security-baseline.sh::nl_eval_attr.
# Required when running inside a sandbox where $ROOT is owned by a
# different uid than the caller.
_STATIC_GITCFG=$(nl_mktemp .static-gitcfg.XXXXXX)
install -d -m 0700 "$_STATIC_GITCFG/git"
printf "[safe]\n\tdirectory = %s\n" "$ROOT" > "$_STATIC_GITCFG/git/config"
export XDG_CONFIG_HOME="$_STATIC_GITCFG"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=safe.directory
export GIT_CONFIG_VALUE_0="$ROOT"

# Shared cache for smoke flake-eval artifacts. Each Layer-1 gate that
# needs the rendered smoke `vms.json` or smoke `bundle.privilegesJson`
# pulls from this cache via lib.sh `nl_smoke_*` helpers; the cache is
# populated lazily by the first caller and reused by the rest.
NL_STATIC_CACHE=$(nl_mktemp .static-cache.XXXXXX)
export NL_STATIC_CACHE

# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> Layer 1: parse + eval"

cd "$ROOT"

# Layout: nixos-modules/ + components/. Old paths
# (modules/nixling/router.nix, modules/nixling/vms.nix,
# modules/nixling/audio.nix, modules/nixling/audio-host.nix,
# modules/nixling/entra-id.nix) are gone:
#   * router.nix renamed to net.nix.
#   * vms.nix is NOT lifted into the public flake (consumers declare
#     their own nixling.vms.<name> bindings).
#   * audio.nix split into components/audio/{guest,host}.nix.
#   * entra-id.nix moved to the sibling entrablau flake.
# Consumer-specific `vms/<name>.nix` paths are excluded — they only
# exist on the maintainer's host. The loop below skips any entry that
# isn't present on disk so the gate stays useful for the public flake
# AND for consumer trees that still carry workload VM definitions.
NL_FILES=(
  nixos-modules/default.nix
  nixos-modules/options.nix
  nixos-modules/options-observability.nix
  nixos-modules/assertions.nix
  nixos-modules/lib.nix
  nixos-modules/base.nix
  nixos-modules/host.nix
  nixos-modules/host-users.nix
  nixos-modules/host-polkit.nix
  nixos-modules/host-activation.nix
  # Removed by +:
  #   nixos-modules/host-wrapper.nix     (nixling@<vm> template)
  #   nixos-modules/host-sidecars.nix    (nixling-<vm>-{gpu,swtpm})
  #   nixos-modules/host-known-hosts.nix (nixling-known-hosts-refresh@)
  #   nixos-modules/host-audit.nix       (nixling-audit-check.{service,timer})
  #   nixos-modules/host-ch-exporter.nix (nixling-ch-exporter.service)
  #   nixos-modules/components/video/host.nix (nixling-<vm>-video)
  nixos-modules/network.nix
  nixos-modules/net.nix
  nixos-modules/observability-vm.nix
  nixos-modules/store.nix
  nixos-modules/components/graphics.nix
  nixos-modules/components/tpm.nix
  nixos-modules/components/usbip.nix
  nixos-modules/components/audit.nix
  nixos-modules/components/home-manager.nix
  nixos-modules/components/audio/guest.nix
  nixos-modules/components/audio/host.nix
  nixos-modules/components/observability/default.nix
  nixos-modules/components/observability/guest.nix
  nixos-modules/components/observability/host.nix
  nixos-modules/components/observability/stack.nix
  tests/smoke-eval-aarch64.nix
  tests/smoke-eval-graphics.nix
  tests/smoke-eval-home-manager.nix
  tests/smoke-eval-extraspecialargs.nix
  tests/smoke-eval-tpm.nix
  flake.nix
)
nl_static_gate_begin "nix-instantiate --parse" "nix-instantiate --parse"
for f in "${NL_FILES[@]}"; do
  if [ ! -f "$ROOT/$f" ]; then
    log "  skip (not present): $f"
    continue
  fi
  if nix-instantiate --parse "$ROOT/$f" >/dev/null 2>&1; then
    ok "parse: $f"
  else
    fail "parse: $f"
  fi
done
nl_static_gate_end "nix-instantiate --parse"

nl_static_gate_begin "shellcheck --severity=warning on all nixling shell scripts" "shellcheck --severity=warning on all nixling shell scripts"
mapfile -t SH_FILES < <(
  find "$ROOT/tests" "$ROOT/scripts" "$ROOT/harness/ubuntu" \
    -maxdepth 1 -name '*.sh' -type f 2>/dev/null | sort
)
if [ "${#SH_FILES[@]}" -eq 0 ]; then
  fail "shellcheck: no .sh files found under tests/ or scripts/"
fi
shellcheck_output_dir=$(nl_mktemp .shellcheck.XXXXXX)
shellcheck_output="$shellcheck_output_dir/output"
if ! command -v shellcheck >/dev/null 2>&1; then
  _STATIC_SHELLCHECK_PATH=$(nix shell --quiet --inputs-from "$ROOT" nixpkgs#shellcheck --command bash -lc 'printf %s "$PATH"')
  _STATIC_SHELLCHECK_PREFIX=$(nl_static_path_prefix "$_STATIC_SHELLCHECK_PATH" "$PATH")
  if [ -n "$_STATIC_SHELLCHECK_PREFIX" ]; then
    PATH="$_STATIC_SHELLCHECK_PREFIX:$PATH"
    export PATH
  fi
fi
if shellcheck --severity=warning -x "${SH_FILES[@]}" >"$shellcheck_output" 2>&1; then
  ok "shellcheck: ${#SH_FILES[@]} files"
else
  head -20 "$shellcheck_output" >&2 || true
  fail "shellcheck: nixling shell scripts"
fi
nl_static_gate_end "shellcheck --severity=warning on all nixling shell scripts"

# v0.2.0 issue #6 — heuristic lint for the NixOS module-system trap
# where one module both declares `mkOption { default = ...; readOnly =
# true; }` and assigns the same `nixling.*` option under `config`. A
# perfect check would need Nix eval introspection; this brace-depth awk
# pass is the pragmatic static gate. It is intentionally heuristic
# (comments / strings can still fool it), but it now keeps `default`
# and `readOnly` bound to the same `mkOption` block even when nested
# attrsets make the option span multiple lines, plus a supplementary
# grep catches one-line `mkOption { default = ...; readOnly = true; }`
# forms that the line-anchored awk attribute checks miss.
#
# store.nix previously declared `nixling.store.package` and
# `nixling.store.generations` as `readOnly + default` internal
# options (the issue-#6 trap pattern). Both were retired in
#  together with the bash CLI. The lint
# below still fails the gate if a future commit re-introduces
# either option on store.nix's `config` surface.
nl_static_gate_begin "readOnly + default + config trio lint" "readOnly + default + config trio lint"
# Heuristic shell-based lint (issue #6). Detects mkOption blocks
# carrying both `readOnly = true` and `default` where the same file
# also assigns `config.nixling.<path>`. Known limitations:
#   - Nested config attribute sets (e.g. `config = { nixling = { ... }; };`)
#     are not detected; only flat `config.nixling.x = ...` assignments.
#   - A Nix-eval-based introspection would be more precise but is
#     deferred until the pattern recurs outside the current allowlist.
TRIO_FAILED=0
TRIO_CONFIG_ASSIGN_RE='^[[:space:]]*(config\.)?nixling\.[A-Za-z0-9_.-]+[[:space:]]*='
STORE_TRIO_ASSIGN_RE='^[[:space:]]*(config\.)?nixling\.store\.(package|generations)[[:space:]]*='
if grep -qE "$STORE_TRIO_ASSIGN_RE" "$ROOT/nixos-modules/store.nix"; then
  grep -nE "$STORE_TRIO_ASSIGN_RE" "$ROOT/nixos-modules/store.nix" >&2 || true
  log "  FAIL: store.nix assigns readOnly+default nixling.store.package/generations"
  TRIO_FAILED=1
fi
while IFS= read -r -d '' nix_file; do
  inline_trio=0
  if grep -n 'mkOption.*default.*readOnly\|mkOption.*readOnly.*default' "$nix_file" >/dev/null \
    && grep -qE "$TRIO_CONFIG_ASSIGN_RE" "$nix_file"; then
    inline_trio=1
  fi

  if {
    awk '
      function brace_delta(s, opens, closes) {
        opens = gsub(/\{/, "{", s)
        closes = gsub(/\}/, "}", s)
        return opens - closes
      }

      {
        line = $0

        if (!in_block && line ~ /mkOption[[:space:]]*\{/) {
          in_block = 1
          depth = 0
          has_readonly = 0
          has_default = 0
        }

        if (in_block) {
          if (line ~ /^[[:space:]]*readOnly[[:space:]]*=[[:space:]]*true[[:space:]]*;/) {
            has_readonly = 1
          }
          if (line ~ /^[[:space:]]*default[[:space:]]*=/) {
            has_default = 1
          }

          depth += brace_delta(line)
          if (depth <= 0) {
            if (has_readonly && has_default) {
              found = 1
              exit 0
            }
            in_block = 0
            depth = 0
          }
        }
      }

      END { exit(found ? 0 : 1) }
    ' "$nix_file" && grep -qE "$TRIO_CONFIG_ASSIGN_RE" "$nix_file"
  } || [ "$inline_trio" -eq 1 ]; then
    grep -nE 'mkOption.*default.*readOnly|mkOption.*readOnly.*default|^[[:space:]]*readOnly[[:space:]]*=|^[[:space:]]*default[[:space:]]*=|^[[:space:]]*(config\.)?nixling\.[A-Za-z0-9_.-]+[[:space:]]*=' "$nix_file" >&2 || true
    log "  FAIL: readOnly+default+config trio detected in $nix_file (issue #6)"
    TRIO_FAILED=1
  fi
done < <(find "$ROOT/nixos-modules" -type f -name '*.nix' -print0 | sort -z)
if [ "$TRIO_FAILED" -eq 0 ]; then
  ok "readOnly + default + config trio lint"
else
  fail "readOnly + default + config trio lint"
fi
nl_static_gate_end "readOnly + default + config trio lint"

nl_static_gate_begin "nix flake check --no-build --all-systems" "nix flake check --no-build --all-systems"
if nix flake check "$ROOT" --no-build --all-systems 2>&1 | tail -20 >> "$NL_LOG"; then
  ok "flake check"
else
  fail "flake check"
fi
nl_static_gate_end "nix flake check --no-build --all-systems"

# Smoke-eval gate. Forces a full module-system evaluation of
# a minimal consumer-style nixosSystem importing nixling.nixosModules.default.
# This catches regressions the bare `flake check` misses, e.g. lazy
# strings inside writeShellApplication that don't fire until the
# module is instantiated against a real config.
#
# These eval-only smoke checks are independent, so run them behind the
# shared NL_STATIC_JOBS semaphore.
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval.nix" \
  "$ROOT/tests/smoke-eval.nix" \
  "let f = import $ROOT/tests/smoke-eval.nix; r = f {}; in r.drvPath" \
  "smoke-eval" \
  20
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval-graphics.nix" \
  "$ROOT/tests/smoke-eval-graphics.nix" \
  "let f = import $ROOT/tests/smoke-eval-graphics.nix; r = f {}; in r.drvPath" \
  "smoke-eval-graphics" \
  20
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval-home-manager.nix" \
  "$ROOT/tests/smoke-eval-home-manager.nix" \
  "let f = import $ROOT/tests/smoke-eval-home-manager.nix; r = f {}; in r.drvPath" \
  "smoke-eval-home-manager" \
  20
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval-extraspecialargs.nix" \
  "$ROOT/tests/smoke-eval-extraspecialargs.nix" \
  "let f = import $ROOT/tests/smoke-eval-extraspecialargs.nix; r = f {}; in r.drvPath" \
  "smoke-eval-extraspecialargs" \
  20
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval-tpm.nix" \
  "$ROOT/tests/smoke-eval-tpm.nix" \
  "let f = import $ROOT/tests/smoke-eval-tpm.nix; r = f {}; in r.drvPath" \
  "smoke-eval-tpm" \
  20
nl_static_parallel_smoke_eval_gate \
  "tests/smoke-eval-aarch64.nix" \
  "$ROOT/tests/smoke-eval-aarch64.nix" \
  "let f = import $ROOT/tests/smoke-eval-aarch64.nix; r = f {}; in r.drvPath" \
  "smoke-eval-aarch64" \
  20
nl_static_parallel_wait_all

# Smaller eval/runtime script gates parallelize well, but the large
# assertion/observability matrices saturate the nix daemon when overlapped.
# Keep those two serial after the lighter fan-out drains.
if [ -x "$ROOT/tests/net-vm-network-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/net-vm-network-eval.sh" "$ROOT/tests/net-vm-network-eval.sh"
fi
if [ -x "$ROOT/tests/bridge-isolation-runtime.sh" ]; then
  nl_static_parallel_script_gate "tests/bridge-isolation-runtime.sh" "$ROOT/tests/bridge-isolation-runtime.sh"
fi
if [ -x "$ROOT/tests/usbip-gating-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/usbip-gating-eval.sh" "$ROOT/tests/usbip-gating-eval.sh"
fi
if [ -x "$ROOT/tests/guest-config-containment-eval.sh" ]; then
  # Asserts the per-VM guest-editable `guestConfigFile` may only set
  # guest OS options, never host-owned microvm.* / nixling.* options.
  nl_static_parallel_script_gate "tests/guest-config-containment-eval.sh" "$ROOT/tests/guest-config-containment-eval.sh"
fi
if [ -x "$ROOT/tests/bridge-ipv6-boot-sysctl-eval.sh" ]; then
  # Asserts every declared bridge has a boot.kernel.sysctl entry that
  # suppresses IPv6 at NixOS activation, closing the boot-time window.
  nl_static_parallel_script_gate "tests/bridge-ipv6-boot-sysctl-eval.sh" "$ROOT/tests/bridge-ipv6-boot-sysctl-eval.sh"
fi
if [ -x "$ROOT/tests/cli-json.sh" ]; then
  nl_static_parallel_script_gate "tests/cli-json.sh" "$ROOT/tests/cli-json.sh"
fi
if [ -x "$ROOT/tests/autostart-wiring-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/autostart-wiring-eval.sh" "$ROOT/tests/autostart-wiring-eval.sh"
fi
if [ -x "$ROOT/tests/restart-policy-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/restart-policy-eval.sh" "$ROOT/tests/restart-policy-eval.sh"
fi
if [ -x "$ROOT/tests/nixlingd-startup-smoke.sh" ]; then
  nl_static_parallel_script_gate "tests/nixlingd-startup-smoke.sh" "$ROOT/tests/nixlingd-startup-smoke.sh"
fi
if [ -x "$ROOT/tests/video-sidecar-hardening-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/video-sidecar-hardening-eval.sh" "$ROOT/tests/video-sidecar-hardening-eval.sh"
fi
if [ -x "$ROOT/tests/video-contract-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/video-contract-eval.sh" "$ROOT/tests/video-contract-eval.sh"
fi
if [ -x "$ROOT/tests/video-binary-contract.sh" ]; then
  nl_static_parallel_script_gate "tests/video-binary-contract.sh" "$ROOT/tests/video-binary-contract.sh"
fi
if [ -x "$ROOT/tests/cli-vm-verbs-eval.sh" ]; then
  # P4fu1 software-r1 / test-r1 closure: wire the cli-vm-verbs Layer-1
  # gate so the bash-fallback removal stays regression-gated.
  nl_static_parallel_script_gate "tests/cli-vm-verbs-eval.sh" "$ROOT/tests/cli-vm-verbs-eval.sh"
fi
if [ -x "$ROOT/tests/cli-nix-consumers-eval.sh" ]; then
  # Regression gate that asserts no
  # consumer of nixos-modules/cli.nix's outputs survives outside the
  # file itself + this gate. Lets the sibling
  # agent delete cli.nix without breaking framework eval.
  nl_static_parallel_script_gate "tests/cli-nix-consumers-eval.sh" "$ROOT/tests/cli-nix-consumers-eval.sh"
fi
if [ -x "$ROOT/tests/broker-caps-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/broker-caps-eval.sh" "$ROOT/tests/broker-caps-eval.sh"
fi
if [ -x "$ROOT/tests/broker-bundle-path-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/broker-bundle-path-eval.sh" "$ROOT/tests/broker-bundle-path-eval.sh"
fi
if [ -x "$ROOT/tests/principal-uid-collision-eval.sh" ]; then
  # v1.2— stablePrincipalId UID-collision eval: asserts
  # every declared principal maps to a unique UID and every UID falls in
  # [50000, 16827215]. Evaluated against examples/multi-env consumer flake.
  nl_static_parallel_script_gate "tests/principal-uid-collision-eval.sh" "$ROOT/tests/principal-uid-collision-eval.sh"
fi
if [ -x "$ROOT/tests/umask-roundtrip-eval.sh" ]; then
  # v1.2— umask end-to-end eval round-trip: asserts
  # swtpm/gpu/audio umask=7 (0o007) propagates from minijail-profiles.nix
  # through processesJson.data without silent pipeline drop.
  nl_static_parallel_script_gate "tests/umask-roundtrip-eval.sh" "$ROOT/tests/umask-roundtrip-eval.sh"
fi
if [ -x "$ROOT/tests/store-overlay-emit-eval.sh" ]; then
  # v1.2— assert DiskInit plan-op emitted in processes.json
  # CH node when writableStoreOverlay is set.
  nl_static_parallel_script_gate "tests/store-overlay-emit-eval.sh" "$ROOT/tests/store-overlay-emit-eval.sh"
fi
if [ -x "$ROOT/tests/volume-mounts-eval.sh" ]; then
  # Declared microvm.volumes must emit stable CH disk serials and matching
  # guest fileSystems entries. Without this, /var stays on tmpfs and
  # identity-bearing services regenerate state on every VM restart.
  nl_static_parallel_script_gate "tests/volume-mounts-eval.sh" "$ROOT/tests/volume-mounts-eval.sh"
fi
if [ -x "$ROOT/tests/tempo-budget-eval.sh" ]; then
  # Static gate for the Tempo retention +
  # sampling budget policy. Asserts Nix-side constants in
  # nixos-modules/components/observability/stack.nix +
  # options-observability.nix stay aligned with
  # docs/reference/tempo-retention-sampling.md.
  nl_static_parallel_script_gate "tests/tempo-budget-eval.sh" "$ROOT/tests/tempo-budget-eval.sh"
fi
if [ -x "$ROOT/tests/daemon-default-compat-eval.sh" ]; then
  # Assert daemonExperimental.enable default
  # flip gate honors readiness + evidence + override semantics.
  nl_static_parallel_script_gate "tests/daemon-default-compat-eval.sh" "$ROOT/tests/daemon-default-compat-eval.sh"
fi
if [ -x "$ROOT/tests/host-validate-verb-eval.sh" ]; then
  # Layer-1 gate for the
  # `nixling host validate --apply` verb that writes per-wave
  # evidence files.
  nl_static_parallel_script_gate "tests/host-validate-verb-eval.sh" "$ROOT/tests/host-validate-verb-eval.sh"
fi
if [ -x "$ROOT/tests/wave-evidence-schema-eval.sh" ]; then
  # Assert the canonical wave
  # evidence schema doc + JSON Schema cover every wave declared
  # in options-daemon.nix.
  nl_static_parallel_script_gate "tests/wave-evidence-schema-eval.sh" "$ROOT/tests/wave-evidence-schema-eval.sh"
fi
if [ -x "$ROOT/tests/polkit-allowlist-eval.sh" ]; then
  # Assert host-polkit.nix names ONLY
  # the daemon-only singleton units (nixlingd.service,
  # nixling-priv-broker.{service,socket}) and contains no
  # references to the retired per-VM / per-env unit shapes.
  nl_static_parallel_script_gate "tests/polkit-allowlist-eval.sh" "$ROOT/tests/polkit-allowlist-eval.sh"
fi
if [ -x "$ROOT/tests/legacy-unit-denylist-eval.sh" ]; then
  # Drift gate enforcing that no
  # systemd unit name retired in reappears in nixos-modules/.
  # EXPECTED-RED until lands; the gate
  # is wired here so the deletion sweep has a machine-checkable
  # target to drive to green.
  nl_static_parallel_script_gate "tests/legacy-unit-denylist-eval.sh" "$ROOT/tests/legacy-unit-denylist-eval.sh"
fi
# ADR index coverage guard (/ -class doc-drift).
if [ -x "$ROOT/tests/adr-index-coverage.sh" ]; then
  nl_static_parallel_script_gate "tests/adr-index-coverage.sh" "$ROOT/tests/adr-index-coverage.sh"
fi
# I3 invariant enforcement (ADR 0022): no new v1.3 deferrals authored
# during v1.2 stabilization. ADR 0022 documents this gate, so it must
# stay wired.
if [ -x "$ROOT/tests/no-new-deferral.sh" ]; then
  nl_static_parallel_script_gate "tests/no-new-deferral.sh" "$ROOT/tests/no-new-deferral.sh"
fi
# Wire the remaining doc/drift gates so
# the clean-break invariants are Layer-1 always-on.
if [ -x "$ROOT/tests/adr-0015-presence-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/adr-0015-presence-eval.sh" "$ROOT/tests/adr-0015-presence-eval.sh"
fi
if [ -x "$ROOT/tests/agents-md-rewrite-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/agents-md-rewrite-eval.sh" "$ROOT/tests/agents-md-rewrite-eval.sh"
fi
if [ -x "$ROOT/tests/privileges-doc-completeness-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/privileges-doc-completeness-eval.sh" "$ROOT/tests/privileges-doc-completeness-eval.sh"
fi
if [ -x "$ROOT/tests/privileges-json-rust-vs-nix-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/privileges-json-rust-vs-nix-eval.sh" "$ROOT/tests/privileges-json-rust-vs-nix-eval.sh"
fi
if [ -x "$ROOT/tests/cli-nix-consumers-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/cli-nix-consumers-eval.sh" "$ROOT/tests/cli-nix-consumers-eval.sh"
fi
# tracing-contract + daemon-metrics need static.sh wiring.
if [ -x "$ROOT/tests/tracing-contract-lint.sh" ]; then
  nl_static_parallel_script_gate "tests/tracing-contract-lint.sh" "$ROOT/tests/tracing-contract-lint.sh"
fi
if [ -x "$ROOT/tests/daemon-metrics-eval.sh" ]; then
  nl_static_parallel_script_gate "tests/daemon-metrics-eval.sh" "$ROOT/tests/daemon-metrics-eval.sh"
fi
# Wire orphaned static-eval gates. These were previously not referenced
# in any CI workflow or aggregator;
# wired here so ci-coverage.sh structural guard passes.
for _d13_gate in \
  audio-argv-shape \
  broker-socket-activation-eval \
  broker-systemd-unit-eval \
  cli-rust-native-host-doctor \
  daemon-autostart-eval \
  daemon-experimental-warning-eval \
  gpu-argv-shape \
  host-prep-dag-eval \
  kernel-modules-parity-eval \
  loki-label-cardinality-eval \
  microvm-nix-absent-eval \
  minijail-validator-audio \
  minijail-validator-cloud-hypervisor \
  minijail-validator-gpu \
  minijail-validator-otel-host-bridge \
  minijail-validator-swtpm \
  minijail-validator-usbip \
  minijail-validator-video \
  minijail-validator-virtiofsd \
  minijail-validator-vsock-relay \
  minijail-validator-wayland-proxy \
  net-vm-bundle-gate-eval \
  niri-vm-borders-eval \
  no-bash-exec-eval \
  otel-acl-migration-eval \
  otel-host-bridge-argv-shape \
  deliverable-gate-inventory \
  per-vm-state-ownership-eval \
  processes-json-eval \
  readiness-waves-eval \
  release-tag-eval \
  ssh-host-key-preflight-eval \
  state-dir-acl-eval \
  store-sync-export-eval \
  stop-dag-reconcile-eval \
  supervisor-option-absent-eval \
  tap-dag-contract-doc-eval \
  usbip-argv-shape \
  usbip-state-machine-eval \
  v1.1-kernel-floor-eval \
  vfsd-watchdog-retired-eval \
  video-argv-shape \
  vm-submodule-cutover-eval \
  vm-submodule-eval; do
  if [ -x "$ROOT/tests/${_d13_gate}.sh" ]; then
    nl_static_parallel_script_gate "tests/${_d13_gate}.sh" "$ROOT/tests/${_d13_gate}.sh"
  fi
done
unset _d13_gate
# ci-coverage.sh structural guard (must run after all other tests
# are registered above so it can attest the full set is wired).
if [ -x "$ROOT/tests/ci-coverage.sh" ]; then
  nl_static_parallel_script_gate "tests/ci-coverage.sh" "$ROOT/tests/ci-coverage.sh"
fi
nl_static_parallel_wait_all

# Gc after smoke-eval + mid-tier eval pool. These two
# clusters together materialize 5+ consumer-config toplevels +
# 7 mid-tier eval gates, each pinning derivations under
# /nix/var/nix/gcroots/auto/. The next phase (assertions +
# observability) builds many more under tryEval; gc'ing first
# keeps the peak well below the watchdog cap.
nl_phase_gc "post-mid-tier-evals"
nl_check_disk_budget "post-mid-tier-evals" || fail "disk budget exhausted after mid-tier eval pool"

nl_static_gate_begin "tests/assertions-eval.sh" "tests/assertions-eval.sh"
if [ -x "$ROOT/tests/assertions-eval.sh" ]; then
  if bash "$ROOT/tests/assertions-eval.sh" >/dev/null 2>&1; then
    ok "assertions-eval"
  else
    bash "$ROOT/tests/assertions-eval.sh" 2>&1 | tail -40 >&2 || true
    fail "assertions-eval"
  fi
fi
nl_static_gate_end "tests/assertions-eval.sh"

nl_static_gate_begin "tests/observability-eval.sh" "tests/observability-eval.sh"
if [ -x "$ROOT/tests/observability-eval.sh" ]; then
  if bash "$ROOT/tests/observability-eval.sh" >/dev/null 2>&1; then
    ok "observability-eval"
  else
    bash "$ROOT/tests/observability-eval.sh" 2>&1 | tail -40 >&2 || true
    fail "observability-eval"
  fi
fi
nl_static_gate_end "tests/observability-eval.sh"

# Release auto-gcroots accumulated by the smoke-eval pool +
# the two big eval gates (assertions/observability). Without this
# the next major phase (manifest-contract + + per-example
# flake-check) stacks its own derivations on top of the ones the
# eval gates pinned, peaking /nix/store growth at ~1.2 TiB. The gc
# costs ~30 s and caps the run-time peak at ~250-400 G.
nl_phase_gc "post-eval-gates"
nl_check_disk_budget "post-eval-gates" || fail "disk budget exhausted after eval gates"

# JSON manifest contract gate. Renders the manifest
# from the same smoke-eval consumer config and validates it against
# docs/reference/manifest-schema.json (JSON Schema Draft 2020-12). Catches:
#   - manifest.nix's computed values drifting from the documented
#     types (e.g. a refactor returning null for a field declared str),
#   - manifest.nix and docs/reference/manifest-schema.json drifting on field
#     names or required-vs-optional status,
#   - reserved `_*`-prefixed keys with an unexpected shape.
#
# Validation runs under nix-shell with python3 + jsonschema; nothing
# else in the test harness depends on Python today, but the jsonschema
# package is small (~50KB) and pulled lazily on first run.
nl_static_gate_begin "manifest JSON contract (docs/reference/manifest-schema.json)" "manifest JSON contract (docs/reference/manifest-schema.json)"
if [ -f "$ROOT/docs/reference/manifest-schema.json" ] && [ -f "$ROOT/tests/smoke-eval.nix" ]; then
  _MANIFEST_DIR=$(nl_mktemp .manifest-gate.XXXXXX)
  _MANIFEST_JSON="$_MANIFEST_DIR/manifest.json"

  # Render the manifest's JSON text via the smoke-eval consumer config.
  # _manifestPkg.text is the bare `builtins.toJSON …` output we ship.
  _RENDER_OK=0
  if nix-instantiate --eval --strict --json --expr "
    let
      pkgs = import <nixpkgs> {};
      lib = pkgs.lib;
      flake = builtins.getFlake "git+file://$ROOT";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          flake.nixosModules.default
          ({ lib, ... }: {
            boot.loader.grub.enable = false;
            boot.loader.systemd-boot.enable = false;
            boot.initrd.includeDefaultModules = false;
            fileSystems.\"/\" = { device = \"tmpfs\"; fsType = \"tmpfs\"; };
            environment.etc.\"machine-id\".text = \"00000000000000000000000000000000\";
            system.stateVersion = \"25.11\";
            users.users.alice = { isNormalUser = true; uid = 1000; };
            nixling.site = { waylandUser = \"alice\"; launcherUsers = [ \"alice\" ]; yubikey.enable = false; };
            nixling.envs.work = { lanSubnet = \"10.20.0.0/24\"; uplinkSubnet = \"192.0.2.0/30\"; };
            nixling.vms.corp-vm = {
              enable = true; env = \"work\"; index = 10; ssh.user = \"alice\";
              config = {
                networking.hostName = lib.mkDefault \"corp-vm\";
                users.users.alice = { isNormalUser = true; uid = 1000; };
              };
            };
          })
        ];
      };
    in nixos.config.nixling._manifestPkg.text
  " 2>/dev/null | jq -r . > "$_MANIFEST_JSON"; then
    _RENDER_OK=1
    ok "manifest-contract: rendered smoke manifest"
  else
    fail "manifest-contract: could not render smoke manifest"
  fi

  if [ "$_RENDER_OK" = "1" ]; then
    # 1. Schema syntactically valid JSON.
    if jq . "$ROOT/docs/reference/manifest-schema.json" >/dev/null 2>&1; then
      ok "manifest-contract: schema JSON syntax"
    else
      fail "manifest-contract: docs/reference/manifest-schema.json is not valid JSON"
    fi

    # 2. Manifest validates against schema (JSON Schema Draft 2020-12).
    #    Also asserts the schema is itself valid Draft 2020-12.
    if nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "
        python3 - <<PYEOF
import json, sys
import jsonschema
schema = json.load(open('$ROOT/docs/reference/manifest-schema.json'))
data = json.load(open('$_MANIFEST_JSON'))
jsonschema.Draft202012Validator.check_schema(schema)
validator = jsonschema.Draft202012Validator(schema)
errors = list(validator.iter_errors(data))
if errors:
    for e in errors:
        print('VALIDATION:', '/'.join(map(str, e.absolute_path)) or '<root>', '->', e.message, file=sys.stderr)
    sys.exit(1)
PYEOF
      " >/dev/null 2>&1; then
      ok "manifest-contract: smoke manifest validates against docs/reference/manifest-schema.json"
    else
      fail "manifest-contract: smoke manifest fails JSON Schema validation"
      nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "
          python3 - <<PYEOF
import json, sys
import jsonschema
schema = json.load(open('$ROOT/docs/reference/manifest-schema.json'))
data = json.load(open('$_MANIFEST_JSON'))
validator = jsonschema.Draft202012Validator(schema)
for e in validator.iter_errors(data):
    print('VALIDATION:', '/'.join(map(str, e.absolute_path)) or '<root>', '->', e.message, file=sys.stderr)
PYEOF
        " 2>&1 | tail -20 >&2 || true
    fi

    # 3. Cross-check: every per-VM field in the smoke manifest must be
    #    declared in the schema's $defs.vmEntry.required list. Catches
    #    the case where manifest.nix gains a new field but the schema
    #    isn't updated. (Schema-required fields missing from the
    #    manifest are caught by the Draft 2020-12 validation above.)
    _SCHEMA_REQUIRED=$(jq -r '.["$defs"].vmEntry.required[]' "$ROOT/docs/reference/manifest-schema.json" | sort -u)
    _MANIFEST_FIELDS=$(jq -r '
      [ .[] | select((type=="object") and (has("name"))) | keys[] ] | unique | .[]
    ' "$_MANIFEST_JSON" | sort -u)
    _UNDOC_FIELDS=$(comm -23 <(printf '%s\n' "$_MANIFEST_FIELDS") <(printf '%s\n' "$_SCHEMA_REQUIRED"))
    if [ -z "$_UNDOC_FIELDS" ]; then
      ok "manifest-contract: all manifest fields documented in schema"
    else
      fail "manifest-contract: undocumented per-VM fields in manifest: $(echo "$_UNDOC_FIELDS" | tr '\n' ' ')"
    fi

    # 4. _manifest.manifestVersion must be present and >= 1 (
    #    locked v1 as the first documented schema).
    _RENDERED_VERSION=$(jq -r '._manifest.manifestVersion // empty' "$_MANIFEST_JSON")
    if [ -n "$_RENDERED_VERSION" ] && [ "$_RENDERED_VERSION" -ge 1 ]; then
      ok "manifest-contract: _manifest.manifestVersion = $_RENDERED_VERSION (>= 1)"
    else
      fail "manifest-contract: _manifest.manifestVersion missing or < 1"
    fi

    # 5. md ↔ json drift detection. The prose schema doc carries a
    #    "Per-VM entry" table whose first column is the field name.
    #    Every field documented in that table must appear in the JSON
    #    Schema's $defs.vmEntry.properties keys, and vice versa. Catches
    #    the case where the .md and .json are edited out of step (e.g.
    #    a field added to the JSON Schema but forgotten in the prose
    #    walkthrough's table).
    _SCHEMA_PROPS=$(jq -r '.["$defs"].vmEntry.properties | keys[]' "$ROOT/docs/reference/manifest-schema.json" | sort -u)
    # The per-VM-entry table lives between the "## Per-VM entry" header
    # and the next "### " sub-section header. Extract its first column
    # (the field name) by:
    #   - keeping only lines starting with `| \``,
    #   - dropping the table-header separator (the `|---` line is
    #     captured by the same prefix filter, then dropped by the awk
    #     pattern below).
    _MD_FIELDS=$(awk '
      /^## Per-VM entry$/ {in_section=1; next}
      in_section && /^### / {in_section=0}
      in_section && /^\| `[a-zA-Z]/ {
        # First column lives between the first pair of backticks.
        if (match($0, /`[^`]+`/)) {
          print substr($0, RSTART+1, RLENGTH-2)
        }
      }
    ' "$ROOT/docs/reference/manifest-schema.md" | sort -u)
    _MD_ONLY=$(comm -23 <(printf '%s\n' "$_MD_FIELDS") <(printf '%s\n' "$_SCHEMA_PROPS"))
    _SCHEMA_ONLY=$(comm -13 <(printf '%s\n' "$_MD_FIELDS") <(printf '%s\n' "$_SCHEMA_PROPS"))
    if [ -z "$_MD_ONLY" ] && [ -z "$_SCHEMA_ONLY" ]; then
      ok "manifest-contract: docs/reference/manifest-schema.{md,json} field inventories match"
    else
      [ -n "$_MD_ONLY" ] && fail "manifest-contract: in manifest-schema.md but missing from manifest-schema.json: $(echo "$_MD_ONLY" | tr '\n' ' ')"
      [ -n "$_SCHEMA_ONLY" ] && fail "manifest-contract: in manifest-schema.json but missing from manifest-schema.md: $(echo "$_SCHEMA_ONLY" | tr '\n' ' ')"
    fi
  fi

  rm -rf -- "$_MANIFEST_DIR"
fi
nl_static_gate_end "manifest JSON contract (docs/reference/manifest-schema.json)"

# The remaining gates evaluate a concrete consumer flake's
# `nixosConfigurations.<NL_HOST_CONFIG>` (default: `desktop`). On a fresh
# clone of the public framework flake, there is no host config — those
# gates simply skip with a SKIP line. On the maintainer's host (or any
# consumer who passes `NL_HOST_CONFIG=<their-host>`), they run as before.
NL_HOST_CONFIG=${NL_HOST_CONFIG:-desktop}
if nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.system.build.toplevel" >/dev/null 2>&1; then
  _HAS_HOST_CONFIG=1
else
  _HAS_HOST_CONFIG=0
  log "  SKIP: per-VM closure eval / dry-build / audio host-flake checks (no nixosConfigurations.$NL_HOST_CONFIG in $ROOT)"
fi

if [ "$_HAS_HOST_CONFIG" = "1" ]; then
  nl_static_gate_begin "per-VM closure eval (.#nixling-<vm>)" "per-VM closure eval (.#nixling-<vm>)"
  # Enumerate VM names from the manifest baked into the CLI. The manifest
  # is exposed via `nixling status` (one VM per line under "vms:"), but
  # the cheapest source is direct nix eval.
  mapfile -t VMS < <(
    nix eval --json \
      "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.nixling.vms" 2>/dev/null \
      | jq -r 'keys[]'
  )
  if [ "${#VMS[@]}" -eq 0 ]; then
    fail "no VMs declared (manifest empty?)"
  fi
  for vm in "${VMS[@]}"; do
    if nix eval --raw "$ROOT#nixling-$vm.outPath" >/dev/null 2>&1; then
      ok "eval: nixling-$vm"
    else
      fail "eval: nixling-$vm"
    fi
  done
  nl_static_gate_end "per-VM closure eval (.#nixling-<vm>)"

  nl_static_gate_begin "nixos-rebuild dry-build" "nixos-rebuild dry-build"
  if sudo -A nixos-rebuild dry-build --flake "$ROOT#$NL_HOST_CONFIG" >/dev/null 2>&1; then
    ok "dry-build"
  else
    fail "dry-build"
  fi
  nl_static_gate_end "nixos-rebuild dry-build"
fi

# -----------------------------------------------------------------------------
# Audio: cheap eval assertions that the audio component is wired
# correctly. We don't enable audio.enable on any VM by default, so most
# of these are presence-of-option checks; for an end-to-end run flip a
# VM's audio.enable = true and re-run.
# -----------------------------------------------------------------------------
nl_static_gate_begin "audio component" "audio component"

if [ "$_HAS_HOST_CONFIG" = "1" ]; then

# 1. The shared systemd-user template unit must be present in the
#    rendered system.
if sudo -A nixos-rebuild build --flake "$ROOT#$NL_HOST_CONFIG" --no-link 2>/dev/null \
     | head -1 >/dev/null; then
  : # nothing — just trigger the build cache
fi

# 2. The audio.enable option must exist on every VM submodule.
if nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.options.nixling.vms.type.getSubOptions.x.audio.enable.declarations" \
     >/dev/null 2>&1 \
   || nix eval --json "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.nixling.vms" 2>/dev/null \
     | jq -e '.[] | has("audio")' >/dev/null 2>&1; then
  ok "audio.enable option declared on nixling.vms.<name>"
else
  fail "audio.enable option missing on nixling.vms.<name>"
fi

SYS=$(nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.system.build.toplevel" 2>/dev/null) || SYS=""
if [ -n "$SYS" ]; then
  # v1 ships a host-side PipeWire client.conf.d stream rule that
  # null-targets vhost-device-sound's INPUT direction when nixling.mic
  # is "off" (and OUTPUT when nixling.speaker is "off") so it doesn't
  # auto-link to host devices uninvited. Note: this is a PipeWire
  # client.conf.d file, NOT a WirePlumber rule — see the placement-
  # notes block in audio-host.nix.
  #
  # security-r8-audio-6: the match key shifted from broad
  # `node.name=vhost-device-sound` + `application.name=~nixling-.*` to
  # per-direction custom props (`nixling.mic`, `nixling.speaker`) so
  # the rule fires ONLY when the corresponding direction is OFF. When
  # mic=on we WANT auto-routing; the old broad rule blocked it
  # forever regardless of audio-state.json.
  PW_RULE="$SYS/etc/pipewire/client.conf.d/90-nixling.conf"
  if [ -e "$PW_RULE" ] \
     && grep -q '"nixling.mic":[[:space:]]*"off"' "$PW_RULE" \
     && grep -q '"nixling.speaker":[[:space:]]*"off"' "$PW_RULE" \
     && grep -q '"target.object":[[:space:]]*"-1"' "$PW_RULE" \
     && grep -q 'stream.rules' "$PW_RULE" \
     && grep -q '"node.dont-fallback":[[:space:]]*true' "$PW_RULE" \
     && grep -q '"node.linger":[[:space:]]*true' "$PW_RULE"; then
    ok "pipewire client stream-rule installed: per-direction nixling.{mic,speaker}=off → target=-1 + dont-fallback + linger"
  else
    fail "pipewire client stream-rule missing or malformed at /etc/pipewire/client.conf.d/90-nixling.conf"
  fi
  if [ -e "$SYS/etc/wireplumber/wireplumber.conf.d/90-nixling.conf" ]; then
    fail "stale wireplumber rule present — should have moved to pipewire client.conf.d"
  else
    ok "no stale wireplumber.conf.d/90-nixling.conf (moved to pipewire client.conf.d)"
  fi
  # nixling-<vm>-snd.service is now a per-VM system service (not user).
  SYS_UNITS=$(find -L "$SYS" -path '*systemd/system*' -name 'nixling-*-snd.service' -print -quit 2>/dev/null || true)
  if [ -n "$SYS_UNITS" ]; then
    ok "nixling-<vm>-snd.service unit(s) present in system closure (system service)"
  else
    fail "no nixling-<vm>-snd.service unit in system closure"
  fi
fi

fi  # end: _HAS_HOST_CONFIG
nl_static_gate_end "audio component"

log "Layer 1 core gates OK"

# -----------------------------------------------------------------------------
# Layer-1 test self-inventory. Keep this before the example/template
# flake checks so adding a new executable Layer-1 tests/*.sh script without
# wiring it into static.sh fails closed.
# -----------------------------------------------------------------------------
nl_static_gate_begin "tests/layer1-self-inventory.sh" "tests/layer1-self-inventory.sh"
if [ -x "$HERE/layer1-self-inventory.sh" ]; then
  if bash "$HERE/layer1-self-inventory.sh" >/dev/null 2>&1; then
    ok "layer1-self-inventory"
  else
    bash "$HERE/layer1-self-inventory.sh" 2>&1 | tail -40 >&2 || true
    fail "layer1-self-inventory"
  fi
fi
nl_static_gate_end "tests/layer1-self-inventory.sh"

# -----------------------------------------------------------------------------
# Rust workspace gate. packages/ lands on the parallel s1 branch, so
# this is a no-op on this isolated s2 branch and becomes a hard gate after the
# Integration merge. tests/stub-no-socket.sh is invoked by
# rust-workspace-checks.sh after the cargo gates.
# -----------------------------------------------------------------------------
nl_static_gate_begin "tests/rust-workspace-checks.sh" "tests/rust-workspace-checks.sh"
if [ -d "$ROOT/packages" ]; then
  if bash "$HERE/rust-workspace-checks.sh" >/dev/null 2>&1; then
    ok "rust-workspace-checks"
  else
    bash "$HERE/rust-workspace-checks.sh" 2>&1 | tail -80 >&2 || true
    fail "rust-workspace-checks"
  fi
else
  log "  no packages/ — skipping rust workspace checks (W0a unstaged)"
fi
nl_static_gate_end "tests/rust-workspace-checks.sh"

# -----------------------------------------------------------------------------
# bundle/schema drift, public vms.json parity, and static portability
# invariants. These scripts skip with a clear log line on isolated branches
# before the DTO/emitter/docs artifacts land, and become hard gates after the
# Integration merge.
# -----------------------------------------------------------------------------
nl_static_gate_begin "W1 bundle/schema static gates" "W1 bundle/schema static gates"
nl_time_begin "W1 smoke cache prewarm"
nl_smoke_vms_json >/dev/null
nl_smoke_bundle_privileges_json >/dev/null
# Prewarm the host.json cache so
# `tests/ifname-nix-rust-parity.sh` running in the parallel pool
# hits the cache instead of triggering a fresh `getFlake` against $ROOT
# while sibling gates (`tests/vms-json-parity.sh`,
# `tests/bundle-drift.sh`) still hold per-test scratch files inside
# $ROOT. Without the prewarm the parity gate fail-closes with
# `path '//$ROOT/.vms-json-parity.XXXXXX' does not exist`.
nl_smoke_bundle_host_json >/dev/null
nl_time_end "W1 smoke cache prewarm"
if [ -x "$HERE/guest-proto-bindings.sh" ]; then
  nl_time_begin "tests/guest-proto-bindings.sh"
  if bash "$HERE/guest-proto-bindings.sh" >/dev/null 2>&1; then
    ok "guest-proto-bindings"
  else
    bash "$HERE/guest-proto-bindings.sh" 2>&1 | tail -80 >&2 || true
    fail "guest-proto-bindings"
  fi
  nl_time_end "tests/guest-proto-bindings.sh"
fi
if [ -x "$HERE/guest-ttrpc-bindings.sh" ]; then
  nl_time_begin "tests/guest-ttrpc-bindings.sh"
  if bash "$HERE/guest-ttrpc-bindings.sh" >/dev/null 2>&1; then
    ok "guest-ttrpc-bindings"
  else
    bash "$HERE/guest-ttrpc-bindings.sh" 2>&1 | tail -80 >&2 || true
    fail "guest-ttrpc-bindings"
  fi
  nl_time_end "tests/guest-ttrpc-bindings.sh"
fi
if [ -x "$HERE/bundle-drift.sh" ]; then nl_static_parallel_script "tests/bundle-drift.sh" "$HERE/bundle-drift.sh"; fi
# host.json per-field schema gold-file drift gate (integrator-wired).
if [ -x "$HERE/host-json-drift-gate.sh" ]; then nl_static_parallel_script "tests/host-json-drift-gate.sh" "$HERE/host-json-drift-gate.sh"; fi
# Assert Nix-emitted ifNameMappings
# pass the Rust looks_nixling_owned format gate.
if [ -x "$HERE/ifname-nix-rust-parity.sh" ]; then nl_static_parallel_script "tests/ifname-nix-rust-parity.sh" "$HERE/ifname-nix-rust-parity.sh"; fi
if [ -x "$HERE/vms-json-parity.sh" ]; then nl_static_parallel_script "tests/vms-json-parity.sh" "$HERE/vms-json-parity.sh"; fi
if [ -x "$HERE/guest-control-proto.sh" ]; then nl_static_parallel_script "tests/guest-control-proto.sh" "$HERE/guest-control-proto.sh"; fi
if [ -x "$HERE/guest-control-auth-nongoals.sh" ]; then nl_static_parallel_script "tests/guest-control-auth-nongoals.sh" "$HERE/guest-control-auth-nongoals.sh"; fi
if [ -x "$HERE/guest-control-vsock-eval.sh" ]; then nl_static_parallel_script "tests/guest-control-vsock-eval.sh" "$HERE/guest-control-vsock-eval.sh"; fi
if [ -x "$HERE/guest-control-vsock-helper-static.sh" ]; then nl_static_parallel_script "tests/guest-control-vsock-helper-static.sh" "$HERE/guest-control-vsock-helper-static.sh"; fi
if [ -x "$HERE/guest-exec-runtime-static.sh" ]; then nl_static_parallel_script "tests/guest-exec-runtime-static.sh" "$HERE/guest-exec-runtime-static.sh"; fi
if [ -x "$HERE/guest-exec-policy-eval.sh" ]; then nl_static_parallel_script "tests/guest-exec-policy-eval.sh" "$HERE/guest-exec-policy-eval.sh"; fi
if [ -x "$HERE/static-invariant-uid0.sh" ]; then nl_static_parallel_script "tests/static-invariant-uid0.sh" "$HERE/static-invariant-uid0.sh"; fi
if [ -x "$HERE/static-invariant-broad-caps.sh" ]; then nl_static_parallel_script "tests/static-invariant-broad-caps.sh" "$HERE/static-invariant-broad-caps.sh"; fi
if [ -x "$HERE/static-invariant-writable-paths.sh" ]; then nl_static_parallel_script "tests/static-invariant-writable-paths.sh" "$HERE/static-invariant-writable-paths.sh"; fi
if [ -x "$HERE/static-invariant-world-readable-leak.sh" ]; then nl_static_parallel_script "tests/static-invariant-world-readable-leak.sh" "$HERE/static-invariant-world-readable-leak.sh"; fi
if [ -x "$HERE/static-invariant-deny-unknown-fields.sh" ]; then nl_static_parallel_script "tests/static-invariant-deny-unknown-fields.sh" "$HERE/static-invariant-deny-unknown-fields.sh"; fi
# -DTO deny_unknown_fields static invariant (integrator-wired).
if [ -x "$HERE/static-invariant-deny-unknown-fields-w3.sh" ]; then nl_static_parallel_script "tests/static-invariant-deny-unknown-fields-w3.sh" "$HERE/static-invariant-deny-unknown-fields-w3.sh"; fi
if [ -x "$HERE/static-invariant-opaque-key-ids.sh" ]; then nl_static_parallel_script "tests/static-invariant-opaque-key-ids.sh" "$HERE/static-invariant-opaque-key-ids.sh"; fi
if [ -x "$HERE/privileges-matrix-completeness.sh" ]; then nl_static_parallel_script "tests/privileges-matrix-completeness.sh" "$HERE/privileges-matrix-completeness.sh"; fi
nl_static_parallel_wait_all
nl_static_gate_end "W1 bundle/schema static gates"

# -----------------------------------------------------------------------------
# Control-plane skeleton gates (per the plan-of-record, plan.md §
# "###: Rust workspace and API skeleton"). These cover the nixling-ipc
# wire types, nixling-priv-broker dispatch, nixlingd socket auth + state
# lock + version negotiation, the Rust-native CLI shim, generated docs +
# error-codes, and bounded fuzz of the manifest_v04 / bundle parsers.
# -----------------------------------------------------------------------------
nl_static_gate_begin "W2 control-plane skeleton gates" "W2 control-plane skeleton gates"
if [ -x "$HERE/static-rust-dependency-direction.sh" ]; then bash "$HERE/static-rust-dependency-direction.sh" || fail "static-rust-dependency-direction"; fi
nl_time_begin "W2 cargo prebuild"
if [ -d "$ROOT/packages" ]; then
  nl_activate_rust_toolchain_path || true
  _W2_WORKSPACE_TARGET=$(nl_cargo_target_dir workspace)
  _W2_BROKER_TARGET=$(nl_cargo_target_dir broker)
  CARGO_TARGET_DIR="$_W2_WORKSPACE_TARGET" cargo build --manifest-path "$ROOT/packages/Cargo.toml" --quiet -p nixling -p nixlingd -p xtask --bins
  CARGO_TARGET_DIR="$_W2_BROKER_TARGET" cargo build --manifest-path "$ROOT/packages/nixling-priv-broker/Cargo.toml" --quiet -p nixling-priv-broker --features layer1-bootstrap
fi
nl_time_end "W2 cargo prebuild"
nl_time_begin "W2 CLI smoke prewarm"
nl_cli_smoke_bundle_tree >/dev/null
nl_cli_smoke_bundle_tree_runner_drift >/dev/null
nl_legacy_cli_bin >/dev/null
nl_time_end "W2 CLI smoke prewarm"
# Group 1: pure/read-only gates that rely on getFlake or generated docs.
# Keep them away from the runtime socket gates below because dirty-tree
# snapshots fail closed on in-repo AF_UNIX socket paths.
if [ -x "$HERE/manifest-v04-roundtrip.sh" ]; then nl_static_parallel_script "tests/manifest-v04-roundtrip.sh" "$HERE/manifest-v04-roundtrip.sh"; fi
if [ -x "$HERE/broker-enum-disposition.sh" ]; then nl_static_parallel_script "tests/broker-enum-disposition.sh" "$HERE/broker-enum-disposition.sh"; fi
if [ -x "$HERE/broker-validate-bundle.sh" ]; then nl_static_parallel_script "tests/broker-validate-bundle.sh" "$HERE/broker-validate-bundle.sh"; fi
# Pin layer1-bootstrap as the default broker feature
# until lands the production-shaped runtime.
if [ -x "$HERE/broker-default-features-build.sh" ]; then nl_static_parallel_script "tests/broker-default-features-build.sh" "$HERE/broker-default-features-build.sh"; fi
if [ -x "$HERE/cli-rust-native-list.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-list.sh" "$HERE/cli-rust-native-list.sh"; fi
if [ -x "$HERE/cli-rust-native-status.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-status.sh" "$HERE/cli-rust-native-status.sh"; fi
if [ -x "$HERE/cli-rust-native-usb.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-usb.sh" "$HERE/cli-rust-native-usb.sh"; fi
if [ -x "$HERE/cli-rust-native-auth-status.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-auth-status.sh" "$HERE/cli-rust-native-auth-status.sh"; fi
if [ -x "$HERE/cli-json-drift.sh" ]; then nl_static_parallel_script "tests/cli-json-drift.sh" "$HERE/cli-json-drift.sh"; fi
if [ -x "$HERE/cli-legacy-bash-dispatch.sh" ]; then nl_static_parallel_script "tests/cli-legacy-bash-dispatch.sh" "$HERE/cli-legacy-bash-dispatch.sh"; fi
if [ -x "$HERE/error-codes-drift.sh" ]; then nl_static_parallel_script "tests/error-codes-drift.sh" "$HERE/error-codes-drift.sh"; fi
if [ -x "$HERE/manpage-completion-drift.sh" ]; then nl_static_parallel_script "tests/manpage-completion-drift.sh" "$HERE/manpage-completion-drift.sh"; fi
if [ -x "$HERE/manpage-completeness-eval.sh" ]; then nl_static_parallel_script "tests/manpage-completeness-eval.sh" "$HERE/manpage-completeness-eval.sh"; fi
# Closure: wire the remaining gates.
if [ -x "$HERE/changelog-v1-cut-eval.sh" ]; then nl_static_parallel_script "tests/changelog-v1-cut-eval.sh" "$HERE/changelog-v1-cut-eval.sh"; fi
if [ -x "$HERE/examples-with-observability-eval.sh" ]; then nl_static_parallel_script "tests/examples-with-observability-eval.sh" "$HERE/examples-with-observability-eval.sh"; fi
if [ -x "$HERE/cli-contract-coverage.sh" ]; then nl_static_parallel_script "tests/cli-contract-coverage.sh" "$HERE/cli-contract-coverage.sh"; fi
if [ -x "$HERE/daemon-api-drift.sh" ]; then nl_static_parallel_script "tests/daemon-api-drift.sh" "$HERE/daemon-api-drift.sh"; fi
nl_static_parallel_wait_all

# Group 2: runtime/socket gates. Their repo-local AF_UNIX sockets make
# getFlake choke on dirty-tree snapshots, so run them only after the
# read-only group has drained.
if [ -x "$HERE/broker-socket-acl.sh" ]; then nl_static_parallel_script "tests/broker-socket-acl.sh" "$HERE/broker-socket-acl.sh"; fi
if [ -x "$HERE/broker-export-audit.sh" ]; then nl_static_parallel_script "tests/broker-export-audit.sh" "$HERE/broker-export-audit.sh"; fi
if [ -x "$HERE/broker-scm-rights-fd-lifecycle.sh" ]; then nl_static_parallel_script "tests/broker-scm-rights-fd-lifecycle.sh" "$HERE/broker-scm-rights-fd-lifecycle.sh"; fi
if [ -x "$HERE/daemon-socket-acl.sh" ]; then nl_static_parallel_script "tests/daemon-socket-acl.sh" "$HERE/daemon-socket-acl.sh"; fi
if [ -x "$HERE/daemon-state-lock.sh" ]; then nl_static_parallel_script "tests/daemon-state-lock.sh" "$HERE/daemon-state-lock.sh"; fi
if [ -x "$HERE/daemon-version-negotiation.sh" ]; then nl_static_parallel_script "tests/daemon-version-negotiation.sh" "$HERE/daemon-version-negotiation.sh"; fi
if [ -x "$HERE/cli-rust-native-audit.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-audit.sh" "$HERE/cli-rust-native-audit.sh"; fi
if [ -x "$HERE/cli-rust-native-host-check.sh" ]; then nl_static_parallel_script "tests/cli-rust-native-host-check.sh" "$HERE/cli-rust-native-host-check.sh"; fi
nl_static_parallel_wait_all
if [ -x "$HERE/manifest-fuzz-bounded.sh" ]; then bash "$HERE/manifest-fuzz-bounded.sh" || fail "manifest-fuzz-bounded"; fi
nl_static_gate_end "W2 control-plane skeleton gates"

# -----------------------------------------------------------------------------
# Host-prepare static gates. Standalone test scripts live under tests/;
# the integrator wires every gate into the parallel-gate pool here. Each
# gate uses tests/lib.sh helpers, writes scratch outside $FLAKE/$ROOT,
# does not create its own flock, and inherits the post-gate nix store gc
# + sccache cleanup.
#
# Carve-out: the `with-entra-id` example-flake check can fail with a
# transient/external crates.io 403 against a `libhimmelblau`-/`kanidm-hsm-
# crypto`-pinned vicondoa/entrablau.nix revision. Set
# `NL_SKIP_WITH_ENTRA_ID=1` to skip the per-example check for that one
# example (the per-example loop honors the knob in the per-example block
# below). Use only after one in-band retry; this is an explicit carve-out
# for an external dependency outage.
# -----------------------------------------------------------------------------
nl_static_gate_begin "W3 host-prepare gates" "W3 host-prepare gates"
# Group 1: pure / read-only gates (cgroup oracle, ifname collision, ioctl
# negatives, kernel-module + device-node matrix, runner-shape preflight,
# minijail version check, ipv6 sysctl readback). Safe to run in parallel
# alongside the fake-backend network gates.
if [ -x "$HERE/cgroup-delegation-oracle.sh" ]; then nl_static_parallel_script "tests/cgroup-delegation-oracle.sh" "$HERE/cgroup-delegation-oracle.sh"; fi
if [ -x "$HERE/pidfd-handoff.sh" ]; then nl_static_parallel_script "tests/pidfd-handoff.sh" "$HERE/pidfd-handoff.sh"; fi
if [ -x "$HERE/host-prepare-network.sh" ]; then nl_static_parallel_script "tests/host-prepare-network.sh" "$HERE/host-prepare-network.sh"; fi
if [ -x "$HERE/ipv6-off-readback.sh" ]; then nl_static_parallel_script "tests/ipv6-off-readback.sh" "$HERE/ipv6-off-readback.sh"; fi
if [ -x "$HERE/ifname-collision.sh" ]; then nl_static_parallel_script "tests/ifname-collision.sh" "$HERE/ifname-collision.sh"; fi
if [ -x "$HERE/path-safety-violation-fs.sh" ]; then nl_static_parallel_script "tests/path-safety-violation-fs.sh" "$HERE/path-safety-violation-fs.sh"; fi
# L3 distro-matrix pin parser/drift gate (integrator-wired).
if [ -x "$HERE/l3-pin-consistency.sh" ]; then nl_static_parallel_script "tests/l3-pin-consistency.sh" "$HERE/l3-pin-consistency.sh"; fi
if [ -x "$HERE/nft-coexistence.sh" ]; then nl_static_parallel_script "tests/nft-coexistence.sh" "$HERE/nft-coexistence.sh"; fi
# Host-prepare idempotency no-op invariant (integrator-wired).
if [ -x "$HERE/host-prepare-idempotency.sh" ]; then nl_static_parallel_script "tests/host-prepare-idempotency.sh" "$HERE/host-prepare-idempotency.sh"; fi
# Ch-net-handoff executable canary (replaces prior doc-grep) (integrator-wired).
if [ -x "$HERE/ch-net-handoff-canary.sh" ]; then nl_static_parallel_script "tests/ch-net-handoff-canary.sh" "$HERE/ch-net-handoff-canary.sh"; fi
if [ -x "$HERE/nft-foreign-rule-preservation.sh" ]; then nl_static_parallel_script "tests/nft-foreign-rule-preservation.sh" "$HERE/nft-foreign-rule-preservation.sh"; fi
if [ -x "$HERE/usbip-firewall-skeleton.sh" ]; then nl_static_parallel_script "tests/usbip-firewall-skeleton.sh" "$HERE/usbip-firewall-skeleton.sh"; fi
if [ -x "$HERE/kernel-module-matrix.sh" ]; then nl_static_parallel_script "tests/kernel-module-matrix.sh" "$HERE/kernel-module-matrix.sh"; fi
if [ -x "$HERE/kernel-module-matrix-eval.sh" ]; then nl_static_parallel_script "tests/kernel-module-matrix-eval.sh" "$HERE/kernel-module-matrix-eval.sh"; fi
if [ -x "$HERE/device-node-matrix.sh" ]; then nl_static_parallel_script "tests/device-node-matrix.sh" "$HERE/device-node-matrix.sh"; fi
if [ -x "$HERE/ioctl-negative.sh" ]; then nl_static_parallel_script "tests/ioctl-negative.sh" "$HERE/ioctl-negative.sh"; fi
if [ -x "$HERE/runner-shape-preflight.sh" ]; then nl_static_parallel_script "tests/runner-shape-preflight.sh" "$HERE/runner-shape-preflight.sh"; fi
# Gates: CH / virtiofsd / swtpm argv generators + DAG executor
# + daemon state-persistence + [pending restart] machinery.
if [ -x "$HERE/ch-argv-shape.sh" ]; then nl_static_parallel_script "tests/ch-argv-shape.sh" "$HERE/ch-argv-shape.sh"; fi
if [ -x "$HERE/virtiofsd-argv-shape.sh" ]; then nl_static_parallel_script "tests/virtiofsd-argv-shape.sh" "$HERE/virtiofsd-argv-shape.sh"; fi
# Layer-1 smoke for the nixling-activation-helper binary (fd-safe
# activation primitives per ADR 0021 + TOCTOU closures).
if [ -x "$HERE/activation-helper-eval.sh" ]; then nl_static_parallel_script "tests/activation-helper-eval.sh" "$HERE/activation-helper-eval.sh"; fi
if [ -x "$HERE/dag-topo.sh" ]; then nl_static_parallel_script "tests/dag-topo.sh" "$HERE/dag-topo.sh"; fi
# GPU / audio / video sidecar argv generators.
if [ -x "$HERE/sidecar-argv-shape.sh" ]; then nl_static_parallel_script "tests/sidecar-argv-shape.sh" "$HERE/sidecar-argv-shape.sh"; fi
# Vsock-relay + USBIP argv generators.
if [ -x "$HERE/w6-argv-shape.sh" ]; then nl_static_parallel_script "tests/w6-argv-shape.sh" "$HERE/w6-argv-shape.sh"; fi
if [ -x "$HERE/minijail-version-check.sh" ]; then nl_static_parallel_script "tests/minijail-version-check.sh" "$HERE/minijail-version-check.sh"; fi
if [ -x "$HERE/multi-env-daemon-backed.sh" ]; then nl_static_parallel_script "tests/multi-env-daemon-backed.sh" "$HERE/multi-env-daemon-backed.sh"; fi
nl_static_parallel_wait_all
if [ -x "$HERE/daemon-state-persistence.sh" ]; then bash "$HERE/daemon-state-persistence.sh" || fail "daemon-state-persistence"; fi
nl_static_gate_end "W3 host-prepare gates"

nl_static_gate_begin "L1c and performance canaries" "L1c and performance canaries"
if [ -x "$HERE/l1c-privilege-oracle.sh" ]; then bash "$HERE/l1c-privilege-oracle.sh" || fail "l1c-privilege-oracle"; fi
if [ -x "$HERE/performance-budgets.sh" ]; then bash "$HERE/performance-budgets.sh" || fail "performance-budgets"; fi
nl_static_gate_end "L1c and performance canaries"

# Gc before per-example flake-check, which is the heaviest
# disk-grower in the gate (each example materializes a full microvm
# toplevel: kernel + initrd + systemd + qemu wrapper, ~150 G across
# 5 examples).
nl_phase_gc "post-w3-gates"
nl_check_disk_budget "post-w3-gates" || fail "disk budget exhausted after W3 host-prepare gates"

#  Runner-shape snapshot regression guards
# (CH variadic argv, absolute vsock paths, /dev/net/tun deviceBind).
nl_static_gate_begin "tests/runner-shape-snapshot.sh" "tests/runner-shape-snapshot.sh"
if [ -x "$HERE/runner-shape-snapshot.sh" ]; then
  if bash "$HERE/runner-shape-snapshot.sh" >/dev/null 2>&1; then
    ok "runner-shape-snapshot"
  else
    bash "$HERE/runner-shape-snapshot.sh" 2>&1 | tail -80 >&2 || true
    fail "runner-shape-snapshot"
  fi
fi
nl_static_gate_end "tests/runner-shape-snapshot.sh"

nl_static_gate_begin "tests/harness-ubuntu-eval.sh" "tests/harness-ubuntu-eval.sh"
if [ -x "$HERE/harness-ubuntu-eval.sh" ]; then
  if bash "$HERE/harness-ubuntu-eval.sh" >/dev/null 2>&1; then
    ok "harness-ubuntu-eval"
  else
    bash "$HERE/harness-ubuntu-eval.sh" 2>&1 | tail -80 >&2 || true
    fail "harness-ubuntu-eval"
  fi
fi
nl_static_gate_end "tests/harness-ubuntu-eval.sh"

# -----------------------------------------------------------------------------
# 7b /— per-example/template flake check. Each `examples/<name>/flake.nix`
# pins `nixling.url = "path:../.."` so this runs the in-tree framework
# without a network fetch. Eval-only (`--no-build --all-systems`); a
# build-level gate already lives in the root flake's
# `checks.<system>.*` (also 7b). `--no-write-lock-file` keeps the
# gate read-only so validation never rewrites an example's pinned lock.
# Adds templates/default/ to the same check surface. Skips gracefully
# if examples/ or templates/default/ don't exist (some downstream consumers
# may strip them).
# -----------------------------------------------------------------------------
nl_static_gate_begin "per-example/template flake check" "per-example/template flake check"
if [ -d "$ROOT/examples/with-entra-id" ] && [ -f "$ROOT/examples/with-entra-id/flake.lock" ] && [ -z "${NL_SKIP_WITH_ENTRA_ID:-}" ]; then
  nl_time_begin "with-entra-id input prewarm"
  _WITH_ENTRA_ID_REF=$(jq -er '.nodes["entrablau"].locked | "github:\(.owner)/\(.repo)/\(.rev)"' "$ROOT/examples/with-entra-id/flake.lock")
  nix build --no-link "$_WITH_ENTRA_ID_REF#checks.x86_64-linux.himmelblau-tpm-drv" >/dev/null
  nl_time_end "with-entra-id input prewarm"
fi
if [ -d "$ROOT/examples" ]; then
  shopt -s nullglob
  for ex in "$ROOT"/examples/*/; do
    [ -f "$ex/flake.nix" ] || continue
    name=$(basename "$ex")
    if [ "$name" = "with-entra-id" ]; then
      continue
    fi
    nl_static_parallel_spawn "example flake check: $name" bash -lc "cd '$ex' && nix flake check --no-build --all-systems --no-write-lock-file"
  done
  shopt -u nullglob
else
  log "  (no examples/ directory — skipping)"
fi
nl_static_parallel_wait_all
if [ -f "$ROOT/examples/with-entra-id/flake.nix" ]; then
  if [ -n "${NL_SKIP_WITH_ENTRA_ID:-}" ]; then
    # Carve-out: explicit operator opt-in to skip the with-entra-id
    # per-example check when its pinned vicondoa/entrablau.nix input
    # fails the cargo fetch with a crates.io 403 against the
    # `libhimmelblau` / `kanidm-hsm-crypto` versions in its lockfile.
    # Used only after the in-band retry below failed.
    log "  example flake check: with-entra-id  skipped via NL_SKIP_WITH_ENTRA_ID=1 (external dependency outage carve-out)"
  else
    nl_time_begin "example flake check: with-entra-id"
    if (cd "$ROOT/examples/with-entra-id" && nix flake check --no-build --all-systems --no-write-lock-file) >/dev/null 2>&1; then
      ok "example flake check: with-entra-id"
    else
      # One in-band retry for the documented transient crates.io 403
      # on libhimmelblau-0.8.18 / kanidm-hsm-crypto-0.3.6 before failing
      # the gate. Set NL_SKIP_WITH_ENTRA_ID=1 to bypass after retries.
      log "  example flake check: with-entra-id  first attempt failed; retrying once (W3 carve-out)"
      if (cd "$ROOT/examples/with-entra-id" && nix flake check --no-build --all-systems --no-write-lock-file) >/dev/null 2>&1; then
        ok "example flake check: with-entra-id (retry)"
      else
        (cd "$ROOT/examples/with-entra-id" && nix flake check --no-build --all-systems --no-write-lock-file) 2>&1 | tail -20 >&2 || true
        fail "example flake check: with-entra-id"
      fi
    fi
    nl_time_end "example flake check: with-entra-id"
  fi
fi
if [ -f "$ROOT/templates/default/flake.nix" ]; then
  template_check_dir=$(nl_mktemp .template-flake-check.XXXXXX)
  cp "$ROOT/templates/default/configuration.nix" "$template_check_dir/configuration.nix"
  sed 's#          ./configuration.nix#          ./configuration.nix\n          ./nixling-static-overrides.nix#' \
    "$ROOT/templates/default/flake.nix" > "$template_check_dir/flake.nix"
  cat > "$template_check_dir/nixling-static-overrides.nix" <<'NIX'
{ lib, ... }:
{
  boot.loader.systemd-boot.enable = lib.mkForce false;
  boot.loader.grub.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = "00000000000000000000000000000000";

  networking.hostName = lib.mkForce "check-template";
  nixling.site.launcherUsers = lib.mkForce [ "check-user" ];
  nixling.site.userAuthorizedKeys = lib.mkForce [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBcheckcheckcheckcheckcheckcheckcheckchecky check@template-check"
  ];

  users.users.check-user = {
    isNormalUser = true;
    uid = 1100;
  };
}
NIX
  (cd "$template_check_dir" && git init -q && git add flake.nix configuration.nix nixling-static-overrides.nix)
  # Run the template flake's own nixosConfigurations
  # wiring, not only the root eval-template check. The copied scratch
  # flake differs only by adding a test-only module that neutralizes
  # intentional TODO sentinels and by overriding nixling to this tree.
  nl_static_parallel_spawn "template flake check: default" bash -lc "cd '$template_check_dir' && nix flake check --no-build --all-systems --no-write-lock-file --override-input nixling 'path:$ROOT'"
else
  log "  (no templates/default/flake.nix — skipping)"
fi
nl_static_parallel_wait_all
nl_static_gate_end "per-example/template flake check"

log "Layer 1 examples/templates OK"
log "Static checks OK"

# Disk-gate-end-gc: the gate realises significant nix store content
# (per-example NixOS toplevels, manifest_v04 closures, smoke artifacts).
# Each `nix shell`, `nix flake check`, and `nix build` registers an
# auto-gcroot under /nix/var/nix/gcroots/auto/ so the realised paths
# can't be GC'd until those roots expire (default: never, until the
# next nix-collect-garbage). Running services on the host already pin
# the /run/current-system closure, so a normal nix-collect-garbage is
# safe.
#
# Without this step, repeated static.sh runs accrete ~1 GiB/run and
# fill the volume after a few iterations (observed at the
# integration boundary today).
#
# Bypass with NL_POST_GATE_GC=0 if you're debugging a gate failure and
# want to inspect the realised paths it left behind.
if [ "${NL_POST_GATE_GC:-1}" != "0" ] && command -v nix-store >/dev/null 2>&1; then
  log "--> nix store gc (release auto-gcroots accumulated by this gate run)"
  # `nix store gc` is the nix-3 spelling; nix-store --gc is the legacy
  # form. Try the new spelling first, fall back to legacy.
  if nix store gc >/dev/null 2>&1; then
    log "  ok: nix store gc completed"
  elif nix-store --gc >/dev/null 2>&1; then
    log "  ok: nix-store --gc completed"
  else
    log "  WARN: nix store gc failed; manually run nix-collect-garbage to reclaim space"
  fi
fi

# Opt-in deep GC of OLD GENERATIONS, not just unreferenced paths.
# `nix store gc` above only removes paths nothing references. Old NixOS
# system generations under /nix/var/nix/profiles/system are auto-gcroots
# so they pin their entire closure forever (today reclaimed 471 GiB by
# pruning 245 stale generations from 2026-05-14..2026-05-24). Operators
# who own the host can enable this:
#
#   NL_POST_GATE_DEEP_GC=1         # user-level generations only (no sudo)
#   NL_POST_GATE_DEEP_GC=1 \
#   NL_POST_GATE_DEEP_GC_SUDO=1    # also prune system generations
#                                  # (requires passwordless sudo for
#                                  # nix-collect-garbage; uses `sudo -n`
#                                  # and skips with a clear log if it
#                                  # can't escalate, never prompts)
#
# Default: 0 (off). This is operator-policy territory; the gate must
# NEVER delete system generations on a developer's host without an
# explicit opt-in. The default `nix store gc` above is enough for CI.
#
# Threshold defaults to 7 days; override with NL_POST_GATE_DEEP_GC_DAYS=N.
if [ "${NL_POST_GATE_DEEP_GC:-0}" = "1" ] && command -v nix-collect-garbage >/dev/null 2>&1; then
  _STATIC_DEEP_GC_DAYS=${NL_POST_GATE_DEEP_GC_DAYS:-7}
  log "--> deep-gc: nix-collect-garbage --delete-older-than ${_STATIC_DEEP_GC_DAYS}d (user profiles)"
  if nix-collect-garbage --delete-older-than "${_STATIC_DEEP_GC_DAYS}d" >/dev/null 2>&1; then
    log "  ok: user-profile deep gc completed"
  else
    log "  WARN: user-profile deep gc failed (continuing)"
  fi
  if [ "${NL_POST_GATE_DEEP_GC_SUDO:-0}" = "1" ]; then
    if sudo -n true 2>/dev/null; then
      log "--> deep-gc: sudo nix-collect-garbage --delete-older-than ${_STATIC_DEEP_GC_DAYS}d (system profile + root channels)"
      if sudo -n nix-collect-garbage --delete-older-than "${_STATIC_DEEP_GC_DAYS}d" >/dev/null 2>&1; then
        log "  ok: system-profile deep gc completed"
      else
        log "  WARN: system-profile deep gc failed (continuing)"
      fi
    else
      log "  SKIP: NL_POST_GATE_DEEP_GC_SUDO=1 set but passwordless sudo unavailable; run manually:"
      log "    sudo nix-collect-garbage --delete-older-than ${_STATIC_DEEP_GC_DAYS}d"
    fi
  fi
fi

# Stop the sccache server before we release the gate
# flock. Cargo spawns sccache as a long-running daemon on first
# RUSTC_WRAPPER=sccache invocation. Sccache forks off the bash that
# cargo ran, INHERITING fd 3 (the lock fd that flock-wrap passed
# into bash). Sccache daemonises itself and stays alive past the
# gate, holding the lock fd open. Subsequent `bash tests/static.sh`
# invocations then block indefinitely on flock even though no
# nixling process is running.
#
# Stop the server explicitly here so its fds close. The server
# will be re-spawned (fresh) on the next gate's first cargo call.
# Bypass with NL_POST_GATE_STOP_SCCACHE=0 if you want to keep the
# in-memory cache warm at the cost of needing to manually kill
# the sccache process before the next run.
if [ "${NL_POST_GATE_STOP_SCCACHE:-1}" != "0" ] && command -v sccache >/dev/null 2>&1; then
  log "--> stop sccache server (close any lock-fd it inherited from cargo)"
  if sccache --stop-server >/dev/null 2>&1; then
    log "  ok: sccache server stopped"
  else
    log "  (sccache --stop-server reported no running server, ok)"
  fi
fi
