# shellcheck shell=bash
# Shared helpers for the nixling test suite.
#
# Each helper is small, dependency-free, and assumes:
#   - We are running on a host with nixling installed and the
#     framework activated (i.e. `nixos-rebuild switch` has happened).
#   - sudo -A works without prompting (the invoking user is in
#     `wheel` and an askPass helper is configured) for tests that
#     touch root-owned state.
#   - `nixling` is on PATH (it's in system.environment, installed by the
#     framework's cli.nix).
#   - jq, ip, ssh are installed (nixpkgs default).
#
# Configurable via env:
#   FLAKE — consumer flake root (default: derived from this lib's
#           location, i.e. the repo containing tests/).
#   NL_OPERATOR_SSH_KEY — host operator's SSH private key for the
#           net-VM root login (default: $HOME/.ssh/id_ed25519).
#
# All output goes to stderr so test functions can `echo` their actual
# return value to stdout if they're producing data.

set -u

# Derive FLAKE from lib.sh's own location (tests/lib.sh → ../).
# Override with FLAKE=/path/to/clone when running against an alien tree.
_LIB_HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
FLAKE=${FLAKE:-$(dirname "$_LIB_HERE")}
# NL_LOG defaults outside $FLAKE so the append churn doesn't race
# `builtins.getFlake (toString $FLAKE)` source captures during
# flake-eval gates. Operators who need a stable in-tree log location
# can still override
# NL_LOG=$FLAKE/.nixling-test.log explicitly.
NL_LOG=${NL_LOG:-${TMPDIR:-/tmp}/nixling-test.$$.log}
# shellcheck disable=SC2034  # STATE_ROOT used by scripts that source this lib
STATE_ROOT=/var/lib/nixling/vms

# Sccache-based cross-worktree dedupe of compiled rustc outputs.
# Default storage location lives outside any single worktree's .cache so
# multiple worktrees share a single rustc-output cache. Override with
# SCCACHE_DIR=... locally to bypass.
export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/nixling-sccache}"
# Cap the on-disk cache so a runaway build can't fill the volume. Tune
# via SCCACHE_CACHE_SIZE in the operator environment if 10 GiB is wrong
# for the host's free-space envelope.
export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-10G}"

nl_repo_root() {
  printf '%s\n' "${ROOT:-${FLAKE:-$(dirname "$_LIB_HERE")}}"
}

# Resolve the flake source as a `git+file://` reference instead of a bare
# path. ALWAYS use this (or the equivalent `git+file://` form) in test eval
# expressions — never `builtins.getFlake (toString $root)`.
#
# Why: a bare path makes Nix use the `path:` fetcher, which copies the
# ENTIRE working tree into the store, including the multi-GiB
# `packages/target` cargo artifacts (measured: ~36 GB / 5+ min per cold
# eval, re-triggered every time a cargo build churns target/). `git+file://`
# copies only git-tracked files (target/ is gitignored), turning a
# 5-minute eval into <1 s.
#
# Semantics: uncommitted edits to TRACKED files are still evaluated
# (dirty-tree). UNTRACKED (never `git add`ed) files are invisible — the
# same contract `nix flake check` already enforces, so "commit before
# building" remains the rule (AGENTS.md "Edit -> commit -> validate").
#
# Purity: `nix-instantiate --eval` is impure by default (works as-is);
# `nix eval` is pure by default and callers MUST pass --impure.
nl_flake_ref() {
  printf 'git+file://%s\n' "${1:-$(nl_repo_root)}"
}

nl_cargo_config_path() {
  case "${1:-workspace}" in
    workspace) printf '%s\n' "$(nl_repo_root)/packages/.cargo/config.toml" ;;
    broker) printf '%s\n' "$(nl_repo_root)/packages/nixling-priv-broker/.cargo/config.toml" ;;
    guest-shell-runner) printf '%s\n' "$(nl_repo_root)/packages/nixling-guest-shell-runner/.cargo/config.toml" ;;
    fuzz) printf '%s\n' "$(nl_repo_root)/packages/nixling-core/fuzz/.cargo/config.toml" ;;
    *)
      fail "unknown cargo target scope: ${1:-<empty>}"
      return 1
      ;;
  esac
}

nl_cargo_target_dir() {
  local scope="${1:-workspace}" config target_dir base
  config=$(nl_cargo_config_path "$scope") || return 1
  if [ ! -f "$config" ]; then
    fail "missing cargo config: $config"
    return 1
  fi
  # Honor an explicit [build].target-dir if present; otherwise use the
  # cargo default ("<workspace-root>/target") for the scope. With the
  # Sccache-based dedup design, target-dir is intentionally NOT
  # set, so each worktree gets its own per-worktree target/ and
  # compiled-output dedup happens cross-worktree via sccache.
  target_dir=$(sed -n 's/^[[:space:]]*target-dir[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$config" | head -1)
  if [ -n "$target_dir" ]; then
    printf '%s\n' "$target_dir"
    return 0
  fi
  case "$scope" in
    workspace) base="$(nl_repo_root)/packages/target" ;;
    broker) base="$(nl_repo_root)/packages/nixling-priv-broker/target" ;;
    guest-shell-runner) base="$(nl_repo_root)/packages/nixling-guest-shell-runner/target" ;;
    fuzz) base="$(nl_repo_root)/packages/nixling-core/fuzz/target" ;;
    *)
      fail "unknown cargo target scope: $scope"
      return 1
      ;;
  esac
  printf '%s\n' "$base"
}

nl_cargo_bin_path() {
  local scope="$1" bin_name="$2" target_dir
  target_dir=$(nl_cargo_target_dir "$scope") || return 1
  printf '%s\n' "$target_dir/debug/$bin_name"
}

nl_prepend_path() {
  local prefix="${1:-}"
  [ -n "$prefix" ] || return 0
  PATH="$prefix:$PATH"
  export PATH
}

nl_activate_rust_toolchain_path() {
  if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
    nl_prepend_path "$NL_RUST_TOOLCHAIN_PATH"
    return 0
  fi
  return 1
}

nl_now_ms() {
  date +%s%3N
}

_nl_time_append() {
  local line="$1" log_path lock_path
  log_path=${NL_STATIC_TIMING_LOG:-}
  [ -n "$log_path" ] || return 0
  lock_path="$log_path.lock"
  if command -v flock >/dev/null 2>&1; then
    exec {lock_fd}>>"$lock_path"
    flock -x "$lock_fd"
    printf '%s\n' "$line" >> "$log_path"
    flock -u "$lock_fd"
    exec {lock_fd}>&-
  else
    printf '%s\n' "$line" >> "$log_path"
  fi
}

nl_time_begin() {
  local label="${1:?nl_time_begin: missing label}"
  local started_ms
  started_ms=$(nl_now_ms)
  _nl_time_append "BEGIN	$label	$started_ms"
}

nl_time_end() {
  local label="${1:?nl_time_end: missing label}"
  local log_path ended_ms started_ms elapsed_ms
  log_path=${NL_STATIC_TIMING_LOG:-}
  [ -n "$log_path" ] || return 0
  ended_ms=$(nl_now_ms)
  started_ms=$(awk -F '\t' -v label="$label" '$1 == "BEGIN" && $2 == label { started = $3 } END { print started }' "$log_path" 2>/dev/null || true)
  if [ -z "$started_ms" ]; then
    started_ms=$ended_ms
  fi
  elapsed_ms=$((ended_ms - started_ms))
  _nl_time_append "END	$label	$ended_ms	$elapsed_ms"
}

# ---------- logging ----------

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$NL_LOG" >&2; }
ok()  { log "  PASS: $*"; }
fail() {
  log "  FAIL: $*"
  return 1
}

# ---------- assertions ----------

assert_eq() {
  local actual="$1" expected="$2" msg="${3:-}"
  if [ "$actual" = "$expected" ]; then
    ok "${msg:-assert_eq} ('$actual')"
  else
    fail "${msg:-assert_eq}: got '$actual', expected '$expected'"
  fi
}

assert_lt() {
  local actual="$1" threshold="$2" msg="${3:-}"
  if [ "$actual" -lt "$threshold" ]; then
    ok "${msg:-assert_lt} ($actual < $threshold)"
  else
    fail "${msg:-assert_lt}: $actual not < $threshold"
  fi
}

assert_ge() {
  local actual="$1" threshold="$2" msg="${3:-}"
  if [ "$actual" -ge "$threshold" ]; then
    ok "${msg:-assert_ge} ($actual >= $threshold)"
  else
    fail "${msg:-assert_ge}: $actual not >= $threshold"
  fi
}

assert_file_exists() {
  local p="$1"
  if [ -e "$p" ]; then
    ok "exists: $p"
  else
    fail "missing file: $p"
  fi
}

assert_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  case "$haystack" in
    *"$needle"*) ok "${msg:-assert_contains} ('...$needle...')" ;;
    *)           fail "${msg:-assert_contains}: '$needle' not in output" ;;
  esac
}

assert_not_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  case "$haystack" in
    *"$needle"*) fail "${msg:-assert_not_contains}: '$needle' WAS in output" ;;
    *)           ok "${msg:-assert_not_contains} ('$needle' absent)" ;;
  esac
}

# ---------- host helpers ----------

host_run() {
  log "  \$ $*"
  "$@"
}

vm_running() {
  # Mirrors cli.nix:vm_pids. Returns 0 if any cloud-hypervisor/qemu
  # process is associated with the named VM via its supervisord
  # cmdline or socket path.
  local vm="$1"
  systemctl is-active --quiet "microvm@${vm}.service" 2>/dev/null && return 0
  pgrep -f "microvm@${vm}\\b|nixos-system-${vm}-" >/dev/null 2>&1
}

# Read the on-disk manifest baked into the nixling derivation. Avoids
# duplicating SSH credential discovery across tests.
vm_ssh_user() {
  jq -r --arg v "$1" '.[$v].sshUser // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}
vm_ssh_key() {
  local vm="$1"
  local override_var="NL_VM_SSH_KEY_${vm//-/_}"
  local override="${!override_var:-}"
  if [ -n "$override" ]; then
    printf '%s\n' "$override"
    return
  fi
  local manifest_key
  manifest_key=$(jq -r --arg v "$vm" '.[$v].sshKeyPath // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null)
  if [ -n "$manifest_key" ]; then
    printf '%s\n' "$manifest_key"
    return
  fi
  local key_dir="${NL_VM_SSH_KEY_DIR:-/var/lib/nixling/keys}"
  local candidate="$key_dir/${vm}_ed25519"
  if [ -r "$candidate" ]; then
    printf '%s\n' "$candidate"
  else
    printf '%s\n' ""
  fi
}
vm_ssh_ip() {
  jq -r --arg v "$1" '.[$v].staticIp // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}

ssh_vm() {
  local vm="$1"; shift
  local user key ip
  user=$(vm_ssh_user "$vm")
  key=$(vm_ssh_key  "$vm")
  ip=$(vm_ssh_ip    "$vm")
  if [ -z "$user" ] || [ -z "$key" ] || [ -z "$ip" ]; then
    fail "ssh_vm: $vm missing ssh.user/ssh.keyPath/staticIp in manifest"
    return 1
  fi
  local kh=/var/lib/nixling/known_hosts.nixling
  ssh -o StrictHostKeyChecking=yes \
      -o UserKnownHostsFile="$kh" \
      -o ConnectTimeout=10 \
      -i "$key" "$user@$ip" "$@"
  local rc=$?
  # security-r8-audio-7: VMs frequently rotate host keys across
  # nixos-rebuilds (the SSH host-key on disk is regenerated in the
  # microvm root). When that happens, ssh fails with 255 + "HOST
  # IDENTIFICATION HAS CHANGED". The L1 known_hosts-refresh service
  # refuses to overwrite a pinned key (security-r7) — operators are
  # expected to rotate manually. For the AUTOMATED TEST suite, we
  # accept that rotation is the norm: remove the stale pin and
  # re-pin via the refresh service, then retry once. This keeps the
  # test resilient to legitimate VM rebuilds without lowering the
  # interactive-shell security posture.
  if [ "$rc" -eq 255 ] && [ -w "$kh" ] || sudo -n -A -- true 2>/dev/null; then
    local ssh_err
    ssh_err=$(ssh -o StrictHostKeyChecking=yes \
                  -o UserKnownHostsFile="$kh" \
                  -o ConnectTimeout=2 -o BatchMode=yes \
                  -i "$key" "$user@$ip" : 2>&1) || true
    if printf '%s' "$ssh_err" | grep -q "HOST IDENTIFICATION HAS CHANGED"; then
      sudo -A ssh-keygen -R "$ip" -f "$kh" >/dev/null 2>&1 || true
      sudo -A systemctl reset-failed "nixling-known-hosts-refresh@${vm}.service" >/dev/null 2>&1 || true
      sudo -A systemctl start "nixling-known-hosts-refresh@${vm}.service" >/dev/null 2>&1 || true
      sleep 1
      ssh -o StrictHostKeyChecking=yes \
          -o UserKnownHostsFile="$kh" \
          -o ConnectTimeout=10 \
          -i "$key" "$user@$ip" "$@"
      return $?
    fi
  fi
  return $rc
}

# ssh_net_vm: reach a net VM (which has no admin user/key in the
# manifest) via root + the host operator's id_ed25519 key, routed over
# the net VM's *uplink* interface IP. The net VM's entry in the
# manifest stores the uplink IP in `.staticIp` (it has no workload-LAN
# address; the workload-side address is the LAN gateway baked into
# net.nix). The net VM's root account accepts the host operator's
# id_ed25519 from the host per net.nix.
#
# Return codes (callers use these to distinguish "infra missing" from
# "SSH failed" so they can SKIP cleanly rather than mis-FAIL):
#   2 — net VM not in manifest / no staticIp
#   3 — operator host key not on disk (net VM build evaluated
#       `lib.optionals (builtins.pathExists ...)` to []; nothing to
#       authenticate with)
#   * — whatever ssh itself returned (255 transport, command exit)
ssh_net_vm() {
  local vm="$1"; shift
  local ip key=${NL_OPERATOR_SSH_KEY:-$HOME/.ssh/id_ed25519}
  ip=$(jq -r --arg v "$vm" \
    '.[$v] | select(.isNetVm == true) | .staticIp // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null)
  if [ -z "$ip" ]; then
    return 2
  fi
  if [ ! -r "$key" ]; then
    return 3
  fi
  ssh -i "$key" \
      -o BatchMode=yes \
      -o ConnectTimeout=5 \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      "root@$ip" "$@"
}

# ---------- cleanup ----------

# Per-process cleanup bookkeeping files MUST live outside $(nl_repo_root)
# so they can't race with `builtins.getFlake (toString <repo>)` source
# captures during a static.sh run. Nix copies the entire flake source
# tree at first eval; if .nl-cleanups.<PID> or .nl-scratch-registry is
# created/removed/appended between the kernel `stat` and the store
# copy, the copy fails with
#   error: path '//<flake source>/.nl-cleanups.<pid>' does not exist
# (observed in cli-legacy-bash-dispatch / cli-json timing-path runs).
#
# Cleanups + scratch registry move to
#   ${NL_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/nixling-bookkeeping}
# which is shared across forks of the same gate run (so the orphan
# reaper at static.sh start can find dead-PID cleanup files there),
# but is invisible to the flake-source enumeration.
NL_BOOKKEEPING_DIR=${NL_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/nixling-bookkeeping}
export NL_BOOKKEEPING_DIR
mkdir -p "$NL_BOOKKEEPING_DIR"

NL_CLEANUP_OWNER_PID=${NL_CLEANUP_OWNER_PID:-$BASHPID}
NL_CLEANUPS=()
NL_CLEANUPS_FILE=${NL_CLEANUPS_FILE:-$NL_BOOKKEEPING_DIR/cleanups.$NL_CLEANUP_OWNER_PID}
: >> "$NL_CLEANUPS_FILE"

add_cleanup() {
  NL_CLEANUPS+=( "$*" )
  printf '%s\n' "$*" >> "$NL_CLEANUPS_FILE"
}

run_cleanups() {
  local i
  local -a cleanup_entries=()
  if [ "$BASHPID" != "$NL_CLEANUP_OWNER_PID" ]; then
    return 0
  fi
  if [ -f "$NL_CLEANUPS_FILE" ]; then
    mapfile -t cleanup_entries < "$NL_CLEANUPS_FILE" || true
  else
    cleanup_entries=( "${NL_CLEANUPS[@]}" )
  fi
  for ((i=${#cleanup_entries[@]}-1; i>=0; i--)); do
    [ -n "${cleanup_entries[$i]}" ] || continue
    log "cleanup: ${cleanup_entries[$i]}"
    eval "${cleanup_entries[$i]}" || log "  (cleanup failed, continuing)"
  done
  rm -f -- "$NL_CLEANUPS_FILE"
}
trap run_cleanups EXIT

nl_scratch_registry_path() {
  printf '%s\n' "$NL_BOOKKEEPING_DIR/scratch-registry"
}

_nl_scratch_unregister() {
  local path="$1" registry entry
  local -a registry_entries=()
  registry=$(nl_scratch_registry_path)
  [ -f "$registry" ] || return 0
  mapfile -t registry_entries < "$registry" || true
  : > "$registry"
  for entry in "${registry_entries[@]}"; do
    [ -n "$entry" ] || continue
    [ "$entry" = "$path" ] && continue
    printf '%s\n' "$entry" >> "$registry"
  done
}

_nl_cleanup_scratch() {
  local path="$1"
  rm -rf -- "$path"
  _nl_scratch_unregister "$path"
}

nl_mktemp() {
  local pattern="${1:?nl_mktemp: missing pattern}" root scratch registry quoted_path
  root=$(nl_repo_root)
  scratch=$(mktemp -d -p "$root" "$pattern") || return 1
  registry=$(nl_scratch_registry_path)
  : >> "$registry"
  printf '%s\n' "$scratch" >> "$registry"
  printf -v quoted_path '%q' "$scratch"
  add_cleanup "_nl_cleanup_scratch $quoted_path"
  printf '%s\n' "$scratch"
}

nl_reap_scratch_orphans() {
  local registry root path
  local -a registry_entries=()
  registry=$(nl_scratch_registry_path)
  [ -f "$registry" ] || return 0
  root=$(nl_repo_root)
  mapfile -t registry_entries < "$registry" || true
  : > "$registry"
  for path in "${registry_entries[@]}"; do
    [ -n "$path" ] || continue
    case "$path" in
      "$root"/*)
        if [ -e "$path" ]; then
          log "reap scratch orphan: $path"
          rm -rf -- "$path"
        fi
        ;;
      *)
        log "skip scratch registry entry outside root: $path"
        ;;
    esac
  done

  # Reap dead-PID cleanups files from the shared bookkeeping dir.
  # Each bash that sources lib.sh creates cleanups.<BASHPID>; if the
  # process died without running its EXIT trap (SIGKILL, crash) the
  # file lingers. Skip the current owner's file and anything whose
  # PID is still alive.
  if [ -d "$NL_BOOKKEEPING_DIR" ]; then
    local f pid
    for f in "$NL_BOOKKEEPING_DIR"/cleanups.*; do
      [ -e "$f" ] || continue
      pid=${f##*/cleanups.}
      case "$pid" in
        ''|*[!0-9]*) continue ;;
      esac
      [ "$pid" = "${NL_CLEANUP_OWNER_PID:-}" ] && continue
      if kill -0 "$pid" 2>/dev/null; then
        continue
      fi
      log "reap dead-pid cleanups file: $f"
      rm -f -- "$f"
    done
  fi
}

# ---------- disk budget + per-phase GC ----------
#
# Full tests/static.sh peak /nix/store growth is ~1.2 TiB cold (per
# the panel-round transcripts). The bulk of that growth is in
# transient derivations (kernel/initrd/systemd toplevels) that are
# only retained via auto-gcroots created by `nix-shell` and
# `nix flake check` inside individual gates. Running a focused
# `nix store gc` between phases reclaims those auto-roots
# immediately, capping the run-time peak at ~250 G and keeping the
# host's shared /nix/store comfortably below the watchdog threshold
# at /tmp/disk-watchdog.sh.
#
# Honor NL_GATE_DISK_BUDGET_GIB (default 0 = unbounded) at each
# phase boundary: if /nix free space drops below this, abort with a
# clear "disk budget exceeded" message rather than wait for the
# emergency watchdog to SIGTERM us mid-derivation.

nl_disk_free_gib() {
  # Query the /nix/store filesystem specifically.
  # The gate's pressure and gc target are /nix/store; on hosts where
  # /nix is a separate mount or subvolume, `df /` would silently
  # report root-fs free space while the store fs is exhausted and
  # the next nix-instantiate fails closed. Always anchor to the
  # actual store path.
  df -BG --output=avail /nix/store 2>/dev/null | tail -1 | tr -dc '0-9'
}

nl_nix_store_used_gib() {
  du -s -BG /nix/store 2>/dev/null | awk '{print $1+0}'
}

nl_phase_gc() {
  local label="$1"
  local before after reclaimed
  before=$(nl_disk_free_gib)
  log "  phase-gc: $label (free before: ${before:-?}G)"
  if ! nix store gc >/dev/null 2>&1; then
    log "  phase-gc: $label: nix store gc returned non-zero (continuing)"
  fi
  after=$(nl_disk_free_gib)
  if [ -n "$before" ] && [ -n "$after" ]; then
    reclaimed=$((after - before))
    log "  phase-gc: $label (free after: ${after}G; reclaimed ~${reclaimed}G)"
  fi
}

nl_check_disk_budget() {
  local label="$1"
  local budget="${NL_GATE_DISK_BUDGET_GIB:-0}"
  if [ "$budget" -le 0 ]; then
    return 0
  fi
  local free
  free=$(nl_disk_free_gib)
  if [ -z "$free" ]; then
    log "  disk-budget: $label: could not read free disk; skipping check"
    return 0
  fi
  if [ "$free" -lt "$budget" ]; then
    log "  FAIL: disk-budget: $label: ${free}G free < ${budget}G NL_GATE_DISK_BUDGET_GIB"
    return 1
  fi
  log "  disk-budget: $label: ${free}G free >= ${budget}G budget"
  return 0
}

# ---------- shared smoke-render cache ----------
#
# Several Layer-1 gates render the same smoke manifest/bundle JSON
# from the flake (e.g. world-readable-leak and opaque-key-ids both
# render `nixling._manifestPkg.text` against the canonical
# work/corp-vm smoke config). Re-rendering it inside each gate
# wastes work, multiplies nix-daemon load, and creates intra-run
# contention that surfaces as transient "could not render smoke
# vms.json" failures when several test runs happen concurrently
# (e.g. integrator + review fleet).
#
# When `tests/static.sh` runs the Layer-1 gates it exports
# `NL_STATIC_CACHE=<scratch dir>` and pre-renders the shared smoke
# artifacts. Each gate calls `nl_smoke_vms_json` / `nl_smoke_bundle_*`
# which lazily render on first request (cached for subsequent
# callers in the same run) and otherwise reuse the cache. When a
# gate runs standalone, the helper still works — it just renders
# into a per-shell fallback scratch dir created here at lib.sh source
# time so that command-substitution callers (e.g. `path=$(nl_smoke_*)`)
# read from a stable directory whose cleanup is tied to the outer
# shell, not to the subshell.
#
# Helpers return the path to the rendered file on stdout. Render
# failures propagate (non-zero exit, caller-visible stderr).
# Always provision a per-process fallback so the cache-dir helper can
# never return an unbound value. The fallback is cheap (empty dir) and
# may go unused when NL_STATIC_CACHE is set and live. Keeping it around
# means a vanishing NL_STATIC_CACHE (e.g. registry race, external rm,
# external cleanup) degrades to a fresh render rather than a `set -u`
# crash. The earlier conditional skipped fallback creation whenever
# NL_STATIC_CACHE was set at lib.sh source-time, which left
# `_NL_SMOKE_FALLBACK` unbound and surfaced as a hard gate failure on
# any code path that fell through `_nl_smoke_cache_dir`'s primary
# branch (observed at the bundle/schema gate after rust-workspace
# completed).
if [ -z "${_NL_SMOKE_FALLBACK:-}" ]; then
  _NL_SMOKE_FALLBACK=$(nl_mktemp .nl-smoke-cache.XXXXXX)
  export _NL_SMOKE_FALLBACK
fi

_nl_smoke_cache_dir() {
  if [ -n "${NL_STATIC_CACHE:-}" ] && [ -d "$NL_STATIC_CACHE" ]; then
    printf '%s\n' "$NL_STATIC_CACHE"
    return 0
  fi
  if [ -n "${NL_STATIC_CACHE:-}" ]; then
    # Primary cache configured but the directory has vanished (registry
    # race, external rm, or a sub-process cleanup that removed it).
    # Try to recreate so callers downstream still see a consistent
    # NL_STATIC_CACHE; if that fails, fall back to the per-process
    # cache. Either way, the helper must never emit an unbound var.
    if mkdir -p "$NL_STATIC_CACHE" 2>/dev/null; then
      printf '%s\n' "$NL_STATIC_CACHE"
      return 0
    fi
  fi
  if [ ! -d "$_NL_SMOKE_FALLBACK" ]; then
    mkdir -p "$_NL_SMOKE_FALLBACK" 2>/dev/null || return 1
  fi
  printf '%s\n' "$_NL_SMOKE_FALLBACK"
}

_nl_smoke_config_modules() {
  cat <<'EOF'
flake.nixosModules.default
        ({ lib, ... }: {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
          environment.etc."machine-id".text =
            "00000000000000000000000000000000";
          system.stateVersion = "25.11";
          users.users.alice = { isNormalUser = true; uid = 1000; };
          nixling.site = {
            waylandUser = "alice";
            launcherUsers = [ "alice" ];
            yubikey.enable = false;
          };
          nixling.envs.work = {
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };
          nixling.vms.corp-vm = {
            enable = true; env = "work"; index = 10; ssh.user = "alice";
            config = {
              networking.hostName = lib.mkDefault "corp-vm";
              users.users.alice = { isNormalUser = true; uid = 1000; };
            };
          };
        })
EOF
}

# Render the smoke `nixling._manifestPkg.text` once per run and emit
# its cached path. Subsequent calls return the same path with no work.
nl_smoke_vms_json() {
  local cache; cache=$(_nl_smoke_cache_dir) || return 1
  local out="$cache/vms.json"
  local err="$cache/vms.json.stderr"
  if [ -s "$out" ]; then
    printf '%s\n' "$out"
    return 0
  fi
  local root="${FLAKE:-${ROOT:-$(pwd)}}"
  local modules; modules=$(_nl_smoke_config_modules)
  local flake_ref; flake_ref=$(nl_flake_ref "$root")
  : > "$err"
  if ! nix-instantiate --eval --strict --json --expr "
    let
      flake = builtins.getFlake \"$flake_ref\";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in nixos.config.nixling._manifestPkg.text
  " 2> "$err" | jq -r . > "$out.tmp"; then
    head -40 "$err" >&2 || true
    rm -f "$out.tmp"
    return 1
  fi
  mv -f "$out.tmp" "$out"
  printf '%s\n' "$out"
}

# Render the smoke `nixling._bundle.privilegesJson.jsonText` once per
# run and emit its cached path.
nl_smoke_bundle_privileges_json() {
  local cache; cache=$(_nl_smoke_cache_dir) || return 1
  local out="$cache/bundle-privileges.json"
  local err="$cache/bundle-privileges.json.stderr"
  if [ -s "$out" ]; then
    printf '%s\n' "$out"
    return 0
  fi
  local root="${FLAKE:-${ROOT:-$(pwd)}}"
  local modules; modules=$(_nl_smoke_config_modules)
  local flake_ref; flake_ref=$(nl_flake_ref "$root")
  : > "$err"
  if ! nix eval --impure --raw --expr "
    let
      flake = builtins.getFlake \"$flake_ref\";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in nixos.config.nixling._bundle.privilegesJson.jsonText
  " > "$out.tmp" 2> "$err"; then
    head -40 "$err" >&2 || true
    rm -f "$out.tmp"
    return 1
  fi
  mv -f "$out.tmp" "$out"
  printf '%s\n' "$out"
}

# Render the smoke `nixling._bundle.hostJson.jsonText` once per run and
# emit its cached path for host-json contract and parity gates.
nl_smoke_bundle_host_json() {
  local cache; cache=$(_nl_smoke_cache_dir) || return 1
  local out="$cache/bundle-host.json"
  local err="$cache/bundle-host.json.stderr"
  if [ -s "$out" ]; then
    printf '%s\n' "$out"
    return 0
  fi
  local root="${FLAKE:-${ROOT:-$(pwd)}}"
  local modules; modules=$(_nl_smoke_config_modules)
  local flake_ref; flake_ref=$(nl_flake_ref "$root")
  : > "$err"
  if ! nix eval --impure --raw --expr "
    let
      flake = builtins.getFlake \"$flake_ref\";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in nixos.config.nixling._bundle.hostJson.jsonText
  " > "$out.tmp" 2> "$err"; then
    head -40 "$err" >&2 || true
    rm -f "$out.tmp"
    return 1
  fi
  mv -f "$out.tmp" "$out"
  printf '%s\n' "$out"
}
