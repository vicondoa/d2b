#!/usr/bin/env bash
#
# migrate-nixling-v0.1.0.sh
#
# HISTORICAL: this script was written for one specific deployment's
# pre-v0.1.0 → v0.1.0 migration. The VM-name arrays below
# (WORKLOAD_VMS, TPM_VMS, NET_VMS_*) and the consumer flake path
# /etc/nixos are baked in. It is preserved here as a reference
# implementation of a TPM-state-preserving rename + unit-disable
# pipeline; for the general nixling onboarding story see
# `templates/default/` and `docs/how-to/migrating-from-microvm.md`.
# If you want to reuse this for your own tree, fork it and adjust
# the arrays. See `scripts/MIGRATION-PRE-V0.1.0.md` for the full
# write-up.
#
# One-shot migration: moves /var/lib/nixling state to the new
# vicondoa/nixling v0.1.0 layout. PRESERVES TPM enrollment.
# Idempotent. Has --dry-run and --rollback.
#
# This script is also distributed inside the public flake at
# `scripts/migrate-nixling-v0.1.0.sh` —
# https://github.com/vicondoa/nixling — so consumers can `nix
# flake archive` or `git clone` the flake and run the script
# directly from the resulting checkout. Path examples below assume
# a typical consumer who imported the flake as input `nixling`:
#
#   sudo -A bash /etc/nixos/scripts/migrate-nixling-v0.1.0.sh --dry-run
#   sudo -A bash /etc/nixos/scripts/migrate-nixling-v0.1.0.sh
#
# Or, if you have the flake checked out under e.g. /etc/nixos:
#
#   sudo -A bash $(nix flake archive --json github:vicondoa/nixling \
#     | jq -r .path)/scripts/migrate-nixling-v0.1.0.sh --dry-run
#
# After success, run `sudo -A nixos-rebuild switch --flake .#desktop`
# in /etc/nixos. Verify TPM enrollment still works with the command
# this script prints at the end.

set -euo pipefail

# -------------------------------------------------------------------
# Configuration
# -------------------------------------------------------------------

# Workload VMs: per-VM state dir under /var/lib/nixling/<vm>/ that
# moves to /var/lib/nixling/vms/<vm>/. Order matters: we move the
# workload VM dir FIRST, then nest the swtpm/<vm>/ inside it.
WORKLOAD_VMS=( work-aad personal-dev nixling-test )

# Workload VMs that hold persistent TPM state. These are the ones
# whose swtpm dirs MUST survive byte-for-byte or Entra-ID will treat
# the device as tampered.
TPM_VMS=( work-aad )

# Net VMs: /var/lib/nixling/<env>-router/ -> /var/lib/nixling/vms/sys-<env>-net/.
# Mirrors the per-VM convention because microvm.nix has a single
# global stateDir: system VMs land under vms/
# alongside workloads, just prefixed with sys-.
NET_VMS_OLD=( work-router personal-router )
NET_VMS_NEW=( sys-work-net sys-personal-net )

STATE_DIR="/var/lib/nixling"
PRIVATE_DIR="/var/lib/private/nixling"
BACKUP_ROOT="/var/lib/nixling-migration-backup"
LOCK_FILE="${STATE_DIR}/.migration.lock"
MARKER_FILE="${STATE_DIR}/.migration-state.json"
IN_PROGRESS_FILE="${STATE_DIR}/.migration-in-progress"
LOG="/var/log/nixling-migration.log"

CURRENT_VERSION=1
TO_VERSION="v0.1.0"
FROM_VERSION="pre-v0.1.0"

# Units to stop+disable. These are the old-naming-convention units
# that will not exist in the new tree.
OLD_INSTANCE_UNITS_PER_VM=(
  "swtpm@%s.service"
  "nixling-snd@%s.service"
  "nixling-gpu@%s.service"
  "nixling-store-sync@%s.service"
)
OLD_NET_VM_UNITS=(
  "microvm@work-router.service"
  "microvm@personal-router.service"
)
OLD_USBIPD_UNITS=(
  "usbipd-nixling.service"
  "usbipd-nixling-work.service"
  "usbipd-nixling-work.socket"
  "usbipd-nixling-work-backend.service"
  "usbipd-nixling-personal.service"
  "usbipd-nixling-personal.socket"
  "usbipd-nixling-personal-backend.service"
)

DRY_RUN=0
ROLLBACK=0
STOP_NET_VMS=0

# Populated at runtime.
TIMESTAMP=""
SNAPSHOT_DIR=""
SNAPSHOT_HASHES_DIR=""
SNAPSHOT_TPM_DIR=""

# -------------------------------------------------------------------
# Logging
# -------------------------------------------------------------------

_log_to_file() {
  if [[ "$DRY_RUN" -eq 0 ]]; then
    # Silent on failure — the script is sometimes invoked as a non-root
    # user (e.g. for --help) where /var/log isn't writable.
    { mkdir -p "$(dirname "$LOG")" && printf '%s\n' "$*" >>"$LOG"; } 2>/dev/null || true
  fi
}

info()  { local m="[INFO]  $*";  printf '%s\n' "$m" >&2; _log_to_file "$m"; }
warn()  { local m="[WARN]  $*";  printf '%s\n' "$m" >&2; _log_to_file "$m"; }
err()   { local m="[ERROR] $*";  printf '%s\n' "$m" >&2; _log_to_file "$m"; }
dry()   { local m="[DRY]   $*";  printf '%s\n' "$m" >&2; _log_to_file "$m"; }
step()  { local m="[STEP]  $*";  printf '\n%s\n' "$m" >&2; _log_to_file "$m"; }

# fail MSG... — fatal in real mode, warning-and-continue in dry-run.
# Pre-flight checks use this so --dry-run shows the full plan even
# when a check would normally abort.
fail() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    warn "(dry-run) pre-flight would FAIL: $*"
    warn "(dry-run) continuing to show the planned actions"
    return 0
  fi
  err "$*"
  exit 1
}

# run CMD...   — execute in normal mode, log-only in dry-run.
run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "$*"
    return 0
  fi
  _log_to_file "[EXEC]  $*"
  "$@"
}

# run_shell "shell pipeline"   — eval a shell pipeline; respects dry-run.
run_shell() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "(shell) $*"
    return 0
  fi
  _log_to_file "[EXEC]  (shell) $*"
  bash -c "$*"
}

usage() {
  cat <<'EOF'
Usage: migrate-nixling-v0.1.0.sh [OPTIONS]

  --dry-run         Show what would be done, modify nothing. Exits 0.
  --rollback        Reverse a previously-applied (or partial) migration.
  --stop-net-vms    If net VMs (work-router, personal-router) are still
                    running, stop them automatically. Without this flag
                    the pre-flight check fails if they're up.
  -h, --help        Show this help.

Run as root via `sudo -A`.
EOF
}

# -------------------------------------------------------------------
# Argument parsing
# -------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)      DRY_RUN=1 ;;
    --rollback)     ROLLBACK=1 ;;
    --stop-net-vms) STOP_NET_VMS=1 ;;
    -h|--help)      usage; exit 0 ;;
    *)              err "Unknown option: $1"; usage; exit 2 ;;
  esac
  shift
done

if [[ "$DRY_RUN" -eq 1 && "$ROLLBACK" -eq 1 ]]; then
  err "--dry-run and --rollback are mutually exclusive."
  exit 2
fi

# -------------------------------------------------------------------
# Lock
# -------------------------------------------------------------------

acquire_lock() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "acquire flock on $LOCK_FILE"
    return 0
  fi
  install -d -m 755 "$STATE_DIR"
  # F8: create the lock file with explicit 0600 mode (root:root) so we
  # don't inherit a permissive umask. `install` is atomic and refuses to
  # clobber, so we only run it if the file doesn't already exist.
  if [[ ! -e "$LOCK_FILE" ]]; then
    install -m 0600 -o root -g root /dev/null "$LOCK_FILE"
  fi
  exec 9<>"$LOCK_FILE"
  if ! flock -n 9; then
    err "Another migration is in progress (lock held on $LOCK_FILE)."
    err ""
    err "Current state: unchanged (this run never started)."
    err ""
    err "What to do:"
    err "  1. Identify the other process:  fuser $LOCK_FILE  ||  lsof $LOCK_FILE"
    err "  2. If it's a leftover from a Ctrl+C'd run that didn't clean up,"
    err "     the lock self-releases when the holder exits. Wait a moment"
    err "     and re-try; if the lock persists with no holder, the process"
    err "     crashed and you can remove it manually:"
    err "       rm -f $LOCK_FILE"
    err "  3. Re-run this script. No --rollback needed."
    exit 1
  fi
}

# -------------------------------------------------------------------
# Marker / in-progress state
# -------------------------------------------------------------------

read_marker_version() {
  # Returns:
  #   - on no marker:          empty string, exit 0
  #   - on parseable marker:   the numeric version, exit 0
  #   - on corrupt marker:     calls fail() with prescriptive recovery
  # F6: a present-but-unparseable marker must NOT be silently treated as
  # "no marker" — that would silently re-run a completed migration.
  if [[ ! -f "$MARKER_FILE" ]]; then
    return 0
  fi
  local v
  v="$(sed -nE 's/.*"migrationVersion"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/p' \
        "$MARKER_FILE" | head -1)"
  if [[ -z "$v" ]]; then
    fail "Marker file is present but unparseable: $MARKER_FILE
This must be resolved manually before the migration can proceed or be rolled back.

Current state: unknown (cannot determine if migration has been applied).

What to do:
  1. Inspect the file:    cat $MARKER_FILE
  2. If the file is empty/zero-bytes, a previous run crashed mid-write.
     Decide based on the actual state of /var/lib/nixling/:
       - If /var/lib/nixling/vms/ exists with the expected VMs, the
         rename completed; restore the marker by hand:
           cat >$MARKER_FILE <<EOF
           {
             \"migrationVersion\": ${CURRENT_VERSION},
             \"appliedAt\": \"<ISO-8601 UTC>\",
             \"fromVersion\": \"${FROM_VERSION}\",
             \"toVersion\": \"${TO_VERSION}\",
             \"snapshotPath\": \"<path to snapshot dir>\"
           }
           EOF
       - If /var/lib/nixling/<vm>/ still exists (pre-migration layout),
         remove the corrupt marker and re-run:
           rm $MARKER_FILE
           sudo -A bash $0 --dry-run     # confirm plan
           sudo -A bash $0               # forward run
  3. If the file looks like JSON but with an unfamiliar shape, it may
     be from a future migration. Do NOT proceed."
    return 0  # in dry-run, fail() does not exit
  fi
  printf '%s' "$v"
}

read_marker_snapshot_path() {
  if [[ -f "$MARKER_FILE" ]]; then
    sed -nE 's/.*"snapshotPath"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' \
      "$MARKER_FILE" | head -1
  fi
}

# F6: write to a temp file in the same directory and rename atomically.
# Both the temp and final paths live on the same filesystem (same dir),
# so `mv -T` is POSIX-atomic.
atomic_write_file() {
  local target="$1" mode="$2" content="$3"
  local dir base tmp
  dir="$(dirname "$target")"
  base="$(basename "$target")"
  install -d -m 755 "$dir"
  # Note: explicit failure check on mktemp — `set -e` does not catch
  # command-substitution failures inside an assignment, so we must
  # validate the result explicitly before setting the cleanup trap.
  if ! tmp="$(mktemp "${dir}/.${base}.tmp.XXXXXX")"; then
    return 1
  fi
  # Guarantee tmp is cleaned up on any failure between here and the
  # final `mv -T`.
  # shellcheck disable=SC2064  # we intentionally expand $tmp at trap-set time
  trap "rm -f '$tmp'" RETURN
  if ! printf '%s\n' "$content" >"$tmp"; then
    return 1
  fi
  if ! chmod "$mode" "$tmp"; then
    return 1
  fi
  if ! mv -T "$tmp" "$target"; then
    return 1
  fi
  trap - RETURN
}

read_in_progress_snapshot() {
  if [[ -f "$IN_PROGRESS_FILE" ]]; then
    sed -nE 's/^snapshotPath=(.*)$/\1/p' "$IN_PROGRESS_FILE" | head -1
  fi
}

write_marker() {
  local snapshot_path="$1"
  local now
  now="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
  local content
  content=$(cat <<EOF
{
  "migrationVersion": ${CURRENT_VERSION},
  "appliedAt": "${now}",
  "fromVersion": "${FROM_VERSION}",
  "toVersion": "${TO_VERSION}",
  "snapshotPath": "${snapshot_path}"
}
EOF
)
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "atomic write marker $MARKER_FILE:"
    dry "$content"
    return 0
  fi
  # F6: atomic write so a crash mid-rename can't leave a half-written
  # marker that would silently fall through on the next run.
  if ! atomic_write_file "$MARKER_FILE" 0644 "$content"; then
    err "Failed to write migration marker: $MARKER_FILE"
    err ""
    err "Current state: migration is COMPLETE but the marker write failed."
    err "This is an idempotency hazard — the next run would re-execute everything."
    err ""
    err "Recovery:"
    err "  1. Verify the rename succeeded by inspecting:"
    err "       ls -la $STATE_DIR/vms/"
    err "  2. Write the marker by hand using the JSON template above (see"
    err "     the dry-run output for an example, or run this script with"
    err "     --dry-run to print it again)."
    err "  3. Do NOT re-run forward — the rename has already happened."
    exit 1
  fi
}

write_in_progress() {
  local snapshot_path="$1"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "atomic write in-progress marker $IN_PROGRESS_FILE (snapshotPath=$snapshot_path)"
    return 0
  fi
  # F6: atomic write. The in-progress marker is the resume anchor; a
  # half-written one would mean we lose track of which snapshot to
  # verify against.
  local content
  content="snapshotPath=${snapshot_path}"
  if ! atomic_write_file "$IN_PROGRESS_FILE" 0644 "$content"; then
    err "Failed to write in-progress marker: $IN_PROGRESS_FILE"
    err ""
    err "Current state: snapshot was created but the resume pointer failed."
    err "Snapshot path: $snapshot_path"
    err ""
    err "Recovery:"
    err "  1. Write the marker manually:"
    err "       echo 'snapshotPath=$snapshot_path' > $IN_PROGRESS_FILE"
    err "  2. Re-run this script (it will resume from the existing snapshot)."
    exit 1
  fi
}

clear_in_progress() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "rm -f $IN_PROGRESS_FILE"
    return 0
  fi
  rm -f "$IN_PROGRESS_FILE"
}

# -------------------------------------------------------------------
# Pre-flight checks
# -------------------------------------------------------------------

check_root() {
  if [[ "$(id -u)" -ne 0 ]]; then
    fail "Must run as root: sudo -A bash $0"
  fi
}

check_clean_git() {
  local repo="/etc/nixos"
  if [[ ! -d "$repo/.git" ]]; then
    fail "$repo is not a git repo."
    return 0
  fi
  local out
  out="$(git -C "$repo" status --porcelain 2>&1 || true)"
  if [[ -n "$out" ]]; then
    if [[ "$DRY_RUN" -eq 1 ]]; then
      warn "(dry-run) /etc/nixos has uncommitted changes — would fail in real mode:"
      printf '%s\n' "$out" >&2
    else
      err "$repo has uncommitted changes:"
      printf '%s\n' "$out" >&2
      err "Commit or stash first. Migration must run against a clean tree so"
      err "any rollback can be paired with a clean rebuild."
      exit 1
    fi
  fi
}

check_unit_inactive() {
  local unit="$1"
  local state
  state="$(systemctl is-active "$unit" 2>/dev/null || true)"
  [[ "$state" == "inactive" || "$state" == "failed" || "$state" == "unknown" || -z "$state" ]]
}

check_workload_vms_stopped() {
  local bad=()
  for vm in "${WORKLOAD_VMS[@]}"; do
    if ! check_unit_inactive "microvm@${vm}.service"; then
      bad+=( "$vm" )
    fi
  done
  if (( ${#bad[@]} > 0 )); then
    local lines="Workload VMs are still running: ${bad[*]}. Stop them first:"
    for vm in "${bad[@]}"; do
      lines+=$'\n'"  nixling down $vm"
    done
    fail "$lines"
  fi
}

check_net_vms_stopped() {
  local running=()
  for vm in "${NET_VMS_OLD[@]}"; do
    if ! check_unit_inactive "microvm@${vm}.service"; then
      running+=( "$vm" )
    fi
  done
  if (( ${#running[@]} == 0 )); then
    return 0
  fi
  if [[ "$STOP_NET_VMS" -eq 1 ]]; then
    info "Stopping net VMs (--stop-net-vms): ${running[*]}"
    for vm in "${running[@]}"; do
      run systemctl stop "microvm@${vm}.service"
    done
    return 0
  fi
  local lines="Net VMs are still running: ${running[*]}. Re-run with --stop-net-vms, or stop manually:"
  for vm in "${running[@]}"; do
    lines+=$'\n'"  systemctl stop microvm@${vm}.service"
  done
  fail "$lines"
}

check_disk_space() {
  if [[ ! -d "$STATE_DIR" ]]; then
    return 0
  fi
  local used_kb free_kb
  # Tolerate non-zero exit from du (e.g., unreadable subdirs as non-root in
  # dry-run) without poisoning the captured number via `|| echo`.
  used_kb="$( { du -sk "$STATE_DIR" 2>/dev/null || true; } | awk '{print $1}')"
  free_kb="$( { df -Pk "$STATE_DIR" 2>/dev/null || true; } | awk 'NR==2 {print $4}')"
  used_kb="${used_kb:-0}"
  free_kb="${free_kb:-0}"
  local needed_kb=$(( used_kb * 2 ))
  # Floor at 2 GB to cover snapshot+working-set even for trivial state dirs.
  if (( needed_kb < 2097152 )); then needed_kb=2097152; fi
  if (( free_kb < needed_kb )); then
    local mountpoint
    mountpoint="$( { df -P "$STATE_DIR" 2>/dev/null || true; } | awk 'NR==2 {print $6}')"
    mountpoint="${mountpoint:-?}"
    fail "Insufficient disk space on $mountpoint: used=$(( used_kb / 1024 ))MiB free=$(( free_kb / 1024 ))MiB required>=$(( needed_kb / 1024 ))MiB

Current state: unchanged (snapshot dir would land here, so we abort before touching anything).

What to do:
  1. Free space on $mountpoint. Likely candidates:
       - Old nixos generations:    sudo -A nix-collect-garbage --delete-older-than 30d
       - Stale microvm builds:     sudo -A nix-store --gc
       - Prior migration snapshots in $BACKUP_ROOT (if any are stale)
  2. Re-run the script (dry-run first):
       sudo -A bash $0 --dry-run"
    return 0
  fi
  info "Disk space OK: $(( free_kb / 1024 )) MiB free, $(( needed_kb / 1024 )) MiB required."
}

check_tools() {
  # F5: Enumerate every external tool the script invokes. Anything called
  # via `run`, `run_shell`, command substitution, or bare invocation is
  # included so a PATH-minimal execution fails up-front rather than
  # mid-rename. Pair each tool with a Nix install hint where appropriate.
  declare -A required_tools=(
    [sha256sum]="coreutils"
    [mv]="coreutils"
    [cp]="coreutils"
    [rm]="coreutils"
    [ln]="coreutils"
    [install]="coreutils"
    [date]="coreutils"
    [du]="coreutils"
    [df]="coreutils"
    [stat]="coreutils"
    [readlink]="coreutils"
    [rmdir]="coreutils"
    [mktemp]="coreutils"
    [tac]="coreutils"
    [wc]="coreutils"
    [dirname]="coreutils"
    [basename]="coreutils"
    [printf]="coreutils"
    [chmod]="coreutils"
    [find]="findutils"
    [xargs]="findutils"
    [diff]="diffutils"
    [awk]="gawk (or busybox awk)"
    [sed]="gnused"
    [grep]="gnugrep"
    [flock]="util-linux"
    [systemctl]="systemd"
    [git]="git"
    [bash]="bash"
  )
  declare -A optional_tools=(
    [tpm2_getcap]="tpm2-tools (only needed if a TPM VM is running at snapshot time)"
    [swtpm_setup]="swtpm (only needed if no TPM VM is running at snapshot time)"
    [ssh]="openssh (only needed if a TPM VM is running at snapshot time)"
  )
  local t missing=() missing_opt=()
  for t in "${!required_tools[@]}"; do
    command -v "$t" >/dev/null 2>&1 || missing+=( "$t" )
  done
  for t in "${!optional_tools[@]}"; do
    command -v "$t" >/dev/null 2>&1 || missing_opt+=( "$t" )
  done
  if (( ${#missing[@]} > 0 )); then
    local lines="Missing required tools (state not yet modified, safe to re-run after fix):"
    local m
    for m in "${missing[@]}"; do
      lines+=$'\n'"  - $m  (install: nix-env -iA nixpkgs.${required_tools[$m]} or add to environment.systemPackages)"
    done
    lines+=$'\n\n'"Add the missing packages to /etc/nixos's environment.systemPackages, rebuild,"
    lines+=$'\n'"then re-run this script. No --rollback needed; no state has been touched."
    fail "$lines"
  fi
  if (( ${#missing_opt[@]} > 0 )); then
    local m
    for m in "${missing_opt[@]}"; do
      warn "optional tool missing: $m  (${optional_tools[$m]})"
    done
  fi
}

preflight() {
  step "Pre-flight checks"
  # check_root has already run in main(); don't repeat (would warn twice
  # under --dry-run).
  check_tools
  check_clean_git
  check_workload_vms_stopped
  check_net_vms_stopped
  check_disk_space
  info "Pre-flight checks passed."
}

# -------------------------------------------------------------------
# Snapshot phase
# -------------------------------------------------------------------

init_snapshot_dir() {
  # If we're resuming a partial migration, reuse the existing snapshot.
  local existing
  existing="$(read_in_progress_snapshot)"
  if [[ -n "$existing" && -d "$existing" ]]; then
    SNAPSHOT_DIR="$existing"
    SNAPSHOT_HASHES_DIR="$SNAPSHOT_DIR/hashes"
    SNAPSHOT_TPM_DIR="$SNAPSHOT_DIR/tpm2_getcap"
    info "Resuming with existing snapshot: $SNAPSHOT_DIR"
    return 0
  fi
  TIMESTAMP="$(date -u +'%Y%m%dT%H%M%SZ')"
  SNAPSHOT_DIR="${BACKUP_ROOT}/${TIMESTAMP}"
  SNAPSHOT_HASHES_DIR="$SNAPSHOT_DIR/hashes"
  SNAPSHOT_TPM_DIR="$SNAPSHOT_DIR/tpm2_getcap"
  run install -d -m 700 "$BACKUP_ROOT"
  run install -d -m 700 "$SNAPSHOT_DIR"
  run install -d -m 700 "$SNAPSHOT_HASHES_DIR"
  run install -d -m 700 "$SNAPSHOT_TPM_DIR"
  write_in_progress "$SNAPSHOT_DIR"
  info "Snapshot dir: $SNAPSHOT_DIR"
}

# hash_dir <src> <out_file>
# Records SHA256 + relative-path of every regular file under <src>,
# sorted so two runs against the same content produce identical output.
# Returns non-zero if the hashing pipeline fails (e.g., unreadable file)
# so cross-FS verification can treat it as fatal.
hash_dir() {
  local src="$1" out="$2"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "hash $src -> $out"
    return 0
  fi
  if [[ ! -d "$src" ]]; then
    : >"$out"
    return 0
  fi
  ( set -o pipefail; cd "$src" && find . -type f -print0 \
      | LC_ALL=C sort -z \
      | xargs -0 -r sha256sum ) >"$out"
}

snapshot_swtpm_state() {
  local vm dir1 dir2
  for vm in "${TPM_VMS[@]}"; do
    dir1="${STATE_DIR}/swtpm/${vm}"
    dir2="${PRIVATE_DIR}/swtpm/${vm}"
    hash_dir "$dir1" "${SNAPSHOT_HASHES_DIR}/${vm}__public.sha256"
    hash_dir "$dir2" "${SNAPSHOT_HASHES_DIR}/${vm}__private.sha256"
    if [[ "$DRY_RUN" -eq 0 ]]; then
      # v0.1.0 BUGFIX: [[ -d ... ]] && info ... under set -e aborts
      # silently when the dir doesn't exist (return-value of compound
      # is treated as function exit status). Use `if` for set-e
      # safety. See https://github.com/vicondoa/nixling/issues/<TBD>.
      if [[ -d "$dir1" ]]; then
        info "Hashed swtpm public  $vm: $(wc -l <"${SNAPSHOT_HASHES_DIR}/${vm}__public.sha256") file(s)"
      fi
      if [[ -d "$dir2" ]]; then
        info "Hashed swtpm private $vm: $(wc -l <"${SNAPSHOT_HASHES_DIR}/${vm}__private.sha256") file(s)"
      fi
    fi
  done
}

snapshot_tpm_getcap() {
  local vm out svc
  for vm in "${TPM_VMS[@]}"; do
    out="${SNAPSHOT_TPM_DIR}/${vm}.txt"
    svc="microvm@${vm}.service"
    if check_unit_inactive "$svc"; then
      if [[ "$DRY_RUN" -eq 1 ]]; then
        dry "VM $vm is stopped — record swtpm_setup --print-capabilities to $out"
      else
        {
          printf 'vm=%s\n' "$vm"
          printf 'state=stopped\n'
          printf 'note=running tpm2_getcap not possible against a stopped VM.\n'
          printf 'note=verify post-rebuild from inside the running VM and compare with hashes/.\n'
          printf -- '--- swtpm_setup --print-capabilities ---\n'
          if command -v swtpm_setup >/dev/null 2>&1; then
            swtpm_setup --print-capabilities 2>&1 || true
          else
            printf 'swtpm_setup not found on PATH\n'
          fi
        } >"$out"
      fi
    else
      if [[ "$DRY_RUN" -eq 1 ]]; then
        dry "VM $vm is running — capture in-guest tpm2_getcap to $out"
      else
        info "VM $vm is running — capturing in-guest tpm2_getcap"
        ssh -o StrictHostKeyChecking=accept-new \
            -o UserKnownHostsFile="${STATE_DIR}/known_hosts.nixling" \
            -i "${STATE_DIR}/${vm}_ed25519" \
            "${vm}.local" "tpm2_getcap properties-fixed" \
          >"$out" 2>&1 || warn "tpm2_getcap over SSH for $vm failed; see $out"
      fi
    fi
  done
}

snapshot_phase() {
  step "Snapshot phase (TPM state preserved before any move)"
  init_snapshot_dir
  # F1 interaction: if we're resuming a previous partial run AFTER any
  # rename was recorded, the existing snapshot is the authoritative
  # reference. Re-hashing now (post-rename) would just capture the new
  # layout and break verification. If renames.tsv is absent/empty, the
  # previous run hadn't moved anything yet — safe to re-hash against
  # the current (post-stop, post-sync) state.
  local renames_file="${SNAPSHOT_DIR}/renames.tsv"
  if [[ -s "$renames_file" ]]; then
    info "Resuming with renames already recorded ($(wc -l <"$renames_file") entries)."
    info "Preserving original snapshot hashes — they are the verification anchor."
  else
    snapshot_swtpm_state
    snapshot_tpm_getcap
  fi
  info "Snapshot complete."
}

# -------------------------------------------------------------------
# Rename phase
# -------------------------------------------------------------------

# same_fs <path-a> <path-b>
# True if both paths are on the same filesystem device. If <path-b>
# doesn't exist, falls back to its nearest existing ancestor.
same_fs() {
  local a="$1" b="$2"
  while [[ ! -e "$b" && "$b" != "/" ]]; do
    b="$(dirname "$b")"
  done
  local da db
  da="$(stat -c '%d' "$a")"
  db="$(stat -c '%d' "$b")"
  [[ "$da" == "$db" ]]
}

# safe_rename <src> <dst>
# Idempotent. Cases:
#   - !src && dst     -> already done, no-op.
#   - src && !dst     -> rename (mv same-fs, else cp -a + rm).
#   - !src && !dst    -> nothing to do (VM never existed), no-op.
#   - src && dst      -> abort. Refuses to merge.
safe_rename() {
  local src="$1" dst="$2"
  if [[ ! -e "$src" && ! -e "$dst" ]]; then
    info "Skip (neither exists): $src"
    return 0
  fi
  if [[ ! -e "$src" && -e "$dst" ]]; then
    info "Skip (already at new path): $dst"
    return 0
  fi
  if [[ -e "$src" && -e "$dst" ]]; then
    err "Refusing to merge: both $src and $dst exist."
    err ""
    err "Current state: this is either a partial result of a failed previous run"
    err "or a manual fix-up gone wrong. The script will not merge directories"
    err "because it cannot distinguish authoritative content from stale content."
    err ""
    err "What to do:"
    err "  1. Inspect both:"
    err "       ls -la $src"
    err "       ls -la $dst"
    err "  2. If $dst is the partial result of a failed previous run:"
    err "       sudo -A bash $0 --rollback"
    err "  3. If $src is the partial result and $dst is the good copy:"
    err "       sudo -A rm -rf $src"
    err "       sudo -A bash $0   # re-run; it should now skip this rename"
    err "  4. If the snapshot dir is gone and rollback can't run, escalate"
    err "     and reconstruct by hand using the renames.tsv pattern."
    exit 1
  fi
  # src exists, dst does not. Create parent of dst, then move.
  local dst_parent
  dst_parent="$(dirname "$dst")"
  run install -d -m 755 "$dst_parent"
  if same_fs "$src" "$dst_parent"; then
    run mv -T "$src" "$dst"
  else
    warn "Cross-filesystem move: $src -> $dst (copy + verify + delete)"
    run cp -a --reflink=auto -T "$src" "$dst"
    # F7: any failure in the temp/hash/diff path must be fatal BEFORE
    # we touch the source. Previously, mktemp failure (e.g., snapshot
    # dir full) would skip the diff check and proceed to `rm -rf $src`
    # — losing data unverified.
    if [[ "$DRY_RUN" -eq 0 ]]; then
      local tmp_src tmp_dst
      if ! tmp_src="$(mktemp -p "$SNAPSHOT_DIR" .crossfs-src.XXXXXX)"; then
        err "FATAL: failed to create temp file in $SNAPSHOT_DIR for cross-FS verify."
        err ""
        err "Current state: $dst now exists (copy succeeded). $src is still intact."
        err "No data lost — but the script will not delete the source unverified."
        err ""
        err "Recovery:"
        err "  1. Free space in $SNAPSHOT_DIR's filesystem:"
        err "       df -h $SNAPSHOT_DIR"
        err "  2. Remove the partial copy at the destination:"
        err "       rm -rf $dst"
        err "  3. Re-run the migration."
        exit 1
      fi
      if ! tmp_dst="$(mktemp -p "$SNAPSHOT_DIR" .crossfs-dst.XXXXXX)"; then
        rm -f "$tmp_src"
        err "FATAL: failed to create second temp file in $SNAPSHOT_DIR for cross-FS verify."
        err ""
        err "Current state: $dst now exists (copy succeeded). $src is still intact."
        err ""
        err "Recovery: same as above — free space, rm -rf $dst, re-run."
        exit 1
      fi
      if ! hash_dir "$src" "$tmp_src"; then
        rm -f "$tmp_src" "$tmp_dst"
        err "FATAL: failed to hash source dir $src for cross-FS verify."
        err "Current state: $dst exists (unverified), $src exists. No data lost."
        err "Recovery: rm -rf $dst, investigate read access on $src, re-run."
        exit 1
      fi
      if ! hash_dir "$dst" "$tmp_dst"; then
        rm -f "$tmp_src" "$tmp_dst"
        err "FATAL: failed to hash destination dir $dst for cross-FS verify."
        err "Current state: $dst exists (unverified), $src exists. No data lost."
        err "Recovery: rm -rf $dst, investigate read access on $dst, re-run."
        exit 1
      fi
      if ! diff -q "$tmp_src" "$tmp_dst" >/dev/null 2>&1; then
        err "FATAL: cross-FS copy hash mismatch: $src vs $dst"
        diff -u "$tmp_src" "$tmp_dst" >&2 || true
        rm -f "$tmp_src" "$tmp_dst"
        err ""
        err "Current state: both source ($src) and destination ($dst) exist."
        err "The copy did not produce byte-identical content. Source is preserved."
        err ""
        err "Recovery:"
        err "  1. Remove the bad copy:    rm -rf $dst"
        err "  2. Investigate why cp produced different bytes (filesystem"
        err "     corruption? ACL/xattr loss? Hardlink farm mismatch?)."
        err "  3. Run --rollback to clean up any prior renames in this run:"
        err "       sudo -A bash $0 --rollback"
        exit 1
      fi
      rm -f "$tmp_src" "$tmp_dst"
    fi
    run rm -rf "$src"
  fi
  printf '%s\t%s\n' "$src" "$dst" \
    | { if [[ "$DRY_RUN" -eq 1 ]]; then cat >&2; else tee -a "${SNAPSHOT_DIR}/renames.tsv" >/dev/null; fi; }
}

rename_workload_vms() {
  local vm
  for vm in "${WORKLOAD_VMS[@]}"; do
    safe_rename "${STATE_DIR}/${vm}" "${STATE_DIR}/vms/${vm}"
  done
}

rename_net_vms() {
  local i old new
  for i in "${!NET_VMS_OLD[@]}"; do
    old="${NET_VMS_OLD[$i]}"
    new="${NET_VMS_NEW[$i]}"
    safe_rename "${STATE_DIR}/${old}" "${STATE_DIR}/vms/${new}"
  done
}

rename_swtpm_public() {
  local vm src dst
  for vm in "${TPM_VMS[@]}"; do
    src="${STATE_DIR}/swtpm/${vm}"
    dst="${STATE_DIR}/vms/${vm}/swtpm"
    # The destination's parent (vms/<vm>) must exist already from the
    # workload rename. safe_rename will install the immediate parent
    # if it doesn't.
    safe_rename "$src" "$dst"
  done
  # If /var/lib/nixling/swtpm/ is now empty, clean it up so it doesn't
  # confuse a future eye.
  if [[ -d "${STATE_DIR}/swtpm" ]]; then
    if [[ -z "$(ls -A "${STATE_DIR}/swtpm" 2>/dev/null)" ]]; then
      run rmdir "${STATE_DIR}/swtpm"
    fi
  fi
}

rename_swtpm_private() {
  local vm src dst
  for vm in "${TPM_VMS[@]}"; do
    src="${PRIVATE_DIR}/swtpm/${vm}"
    dst="${PRIVATE_DIR}/vms/${vm}/swtpm"
    safe_rename "$src" "$dst"
  done
  if [[ -d "${PRIVATE_DIR}/swtpm" ]]; then
    if [[ -z "$(ls -A "${PRIVATE_DIR}/swtpm" 2>/dev/null)" ]]; then
      run rmdir "${PRIVATE_DIR}/swtpm"
    fi
  fi
}

rename_phase() {
  # F1: sidecar stop + sync now happen in pre_rename_stop_phase BEFORE
  # the snapshot, so any in-flight TPM writes are already settled by the
  # time we hash. Here we only do the actual renames + a post-rename sync.
  step "Rename phase (state dirs -> new layout)"
  rename_workload_vms
  rename_net_vms
  rename_swtpm_public
  rename_swtpm_private
  step "sync — flushing pending writes before verification"
  run sync
  info "Rename complete."
}

# F3: identify the sidecars whose stop/disable failure MUST be fatal
# (they hold open handles on state we're about to move byte-for-byte).
is_critical_sidecar() {
  local unit="$1"
  case "$unit" in
    swtpm@*.service \
    | microvm-virtiofsd@*.service \
    | nixling-gpu@*.service)
      return 0 ;;
    *) return 1 ;;
  esac
}

fail_critical_stop() {
  local svc="$1" reason="$2"
  err ""
  err "FATAL: critical sidecar would not stop: $svc"
  err "  reason: $reason"
  err ""
  err "Current state: no state directories have been renamed yet. NO DATA LOSS."
  err ""
  err "Why this is fatal: $svc holds open file handles on TPM / virtiofs / GPU"
  err "state that we're about to mv. If we proceed while it's running, we risk"
  err "corrupting persistent TPM enrollment — Entra-ID will treat the device"
  err "as tampered and Intune will force re-enrollment + raise a security alert."
  err ""
  err "What to do:"
  err "  1. Inspect the unit and its journal:"
  err "       systemctl status $svc"
  err "       journalctl -xeu $svc --no-pager | tail -200"
  err "  2. Try a graceful stop, then a forceful one:"
  err "       systemctl stop $svc"
  err "       systemctl kill --signal=SIGKILL $svc"
  err "       systemctl reset-failed $svc"
  err "  3. Confirm it is now inactive:"
  err "       systemctl is-active $svc        # should print: inactive"
  err "  4. Re-run this script. --rollback is NOT needed (nothing was moved)."
  err ""
  exit 1
}

# stop_unit_or_fail <unit>
# Stops the unit if active. Failure to stop is FATAL for critical
# sidecars (F3), a warning for non-critical sidecars (USBIPD, audio,
# store-sync). "Unit does not exist" is always a benign no-op.
stop_unit_or_fail() {
  local svc="$1"
  if ! unit_exists "$svc"; then
    return 0
  fi
  if check_unit_inactive "$svc"; then
    return 0
  fi
  info "Stopping: $svc"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "systemctl stop $svc"
    return 0
  fi
  if ! systemctl stop "$svc"; then
    if is_critical_sidecar "$svc"; then
      fail_critical_stop "$svc" "systemctl stop returned non-zero"
    fi
    warn "stop failed for $svc (non-critical, continuing)"
    return 0
  fi
  # Some systemd versions return 0 even when ExecStop hangs and the unit
  # ends up in failed/activating limbo. Verify post-condition explicitly.
  if ! check_unit_inactive "$svc"; then
    if is_critical_sidecar "$svc"; then
      fail_critical_stop "$svc" "systemctl stop returned 0 but unit is still active/activating"
    fi
    warn "$svc still active after stop (non-critical, continuing)"
  fi
}

# F4: enumerate USBIP units under both the OLD naming (singleton +
# usbipd-nixling-*) and the NEW naming introduced by
# (nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}). On a
# partially-migrated host (e.g., one where the user already merged a
# When a new flake commit has already been applied before running this script), the new units may
# be present and active, holding the listener ports we need free.
discover_new_usbipd_units() {
  # Skip privileged enumeration in dry-run as non-root — `systemctl
  # list-units` is fine but the patterns might match nothing on the
  # author's host; we still want to print the discovery step.
  if ! command -v systemctl >/dev/null 2>&1; then
    return 0
  fi
  systemctl list-units --all --plain --no-legend --no-pager \
      'nixling-sys-*-usbipd-*.service' \
      'nixling-sys-*-usbipd-*.socket' \
      2>/dev/null \
    | awk '{print $1}' \
    | grep -E '^nixling-sys-.*-usbipd-.*\.(service|socket)$' \
    | LC_ALL=C sort -u || true
}

# Stop every sidecar that may hold an open handle on a state dir we're
# about to move. Critical units (swtpm/virtiofsd/gpu) abort the run on
# stop failure; everything else warns. Idempotent: re-running after a
# partial-fail is safe.
stop_old_sidecars() {
  local vm svc tmpl
  local svcs_per_vm=(
    "swtpm@%s.service"
    "microvm-virtiofsd@%s.service"
    "nixling-gpu@%s.service"
    "nixling-snd@%s.service"
    "nixling-store-sync@%s.service"
    "nixling-known-hosts-refresh@%s.service"
  )
  local all_vms=( "${WORKLOAD_VMS[@]}" "${NET_VMS_OLD[@]}" )
  for vm in "${all_vms[@]}"; do
    for tmpl in "${svcs_per_vm[@]}"; do
      # tmpl is a hard-coded format string like "swtpm@%s.service" — use
      # bash substitution rather than printf to avoid SC2059 (variable
      # in printf format string).
      svc="${tmpl//%s/$vm}"
      stop_unit_or_fail "$svc"
    done
  done
  # Old-naming USBIPD: the singleton + per-env proxies from the pre-
  # design. Non-critical (no TPM state held), so warn-on-failure.
  for svc in "${OLD_USBIPD_UNITS[@]}"; do
    stop_unit_or_fail "$svc"
  done
  # F4: new-naming USBIPD units that may already be loaded on a
  # partially-migrated host. Discover dynamically — the env list isn't
  # hardcoded here because future flake changes might add more.
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "enumerate nixling-sys-*-usbipd-* units and stop any that are active"
  else
    local nu
    while IFS= read -r nu; do
      [[ -z "$nu" ]] && continue
      stop_unit_or_fail "$nu"
    done < <(discover_new_usbipd_units)
  fi
}

# F1: pre-rename phase. Stops every sidecar that could mutate state
# during the snapshot/rename window, then syncs the disk so any
# in-flight writes are durable before we hash.
pre_rename_stop_phase() {
  step "Pre-rename stop phase (stop sidecars before snapshot — F1 fix for the swtpm-outlives-microvm race)"
  stop_old_sidecars
  step "sync — flushing pending writes before snapshot"
  run sync
}

# -------------------------------------------------------------------
# Verification phase
# -------------------------------------------------------------------

verify_swtpm_hashes() {
  local vm new_pub new_priv expect_pub expect_priv tmp_pub tmp_priv ok=1
  for vm in "${TPM_VMS[@]}"; do
    expect_pub="${SNAPSHOT_HASHES_DIR}/${vm}__public.sha256"
    expect_priv="${SNAPSHOT_HASHES_DIR}/${vm}__private.sha256"
    new_pub="${STATE_DIR}/vms/${vm}/swtpm"
    new_priv="${PRIVATE_DIR}/vms/${vm}/swtpm"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      dry "verify $new_pub  against $expect_pub"
      dry "verify $new_priv against $expect_priv"
      continue
    fi

    tmp_pub="$(mktemp -p "$SNAPSHOT_DIR" .verify-pub.XXXXXX)"
    tmp_priv="$(mktemp -p "$SNAPSHOT_DIR" .verify-priv.XXXXXX)"
    hash_dir "$new_pub"  "$tmp_pub"
    hash_dir "$new_priv" "$tmp_priv"

    # File-vs-file diff. Empty source = empty expect = match.
    if ! diff -q "$expect_pub" "$tmp_pub" >/dev/null 2>&1; then
      err "TPM HASH MISMATCH: $vm public swtpm state."
      err "  expected: $expect_pub"
      err "  actual:   $tmp_pub"
      diff -u "$expect_pub" "$tmp_pub" >&2 || true
      ok=0
    else
      info "TPM hash OK: $vm public  ($(wc -l <"$expect_pub") file(s))"
    fi
    if ! diff -q "$expect_priv" "$tmp_priv" >/dev/null 2>&1; then
      err "TPM HASH MISMATCH: $vm private swtpm state."
      err "  expected: $expect_priv"
      err "  actual:   $tmp_priv"
      diff -u "$expect_priv" "$tmp_priv" >&2 || true
      ok=0
    else
      info "TPM hash OK: $vm private ($(wc -l <"$expect_priv") file(s))"
    fi
    rm -f "$tmp_pub" "$tmp_priv"
  done
  if [[ "$ok" -ne 1 ]]; then
    err ""
    err "ABORTING. TPM state did not survive the rename byte-for-byte."
    err "Run with --rollback IMMEDIATELY to restore the pre-migration layout:"
    err "  sudo -A bash $0 --rollback"
    err ""
    err "Do NOT proceed to nixos-rebuild. Do NOT boot the work-aad VM in"
    err "this state — Entra-ID will treat it as device tampering and"
    err "force re-enrollment."
    exit 1
  fi
}

verification_phase() {
  step "Verification phase (TPM hashes must match snapshot)"
  verify_swtpm_hashes
  info "Verification passed. TPM state is byte-identical at new paths."
}

# -------------------------------------------------------------------
# Unit-disable phase
# -------------------------------------------------------------------

unit_exists() {
  # `list-unit-files` only matches static unit files; templated
  # instances like `swtpm@work-aad.service` don't have a file, only
  # the parent template does. `systemctl cat` works for both.
  systemctl cat -- "$1" >/dev/null 2>&1
}

unit_enabled() {
  local state
  state="$(systemctl is-enabled "$1" 2>/dev/null || true)"
  [[ "$state" == "enabled" || "$state" == "alias" || "$state" == "static" || "$state" == "enabled-runtime" ]]
}

record_disabled_unit() {
  local unit="$1"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "record disabled unit: $unit"
    return 0
  fi
  printf '%s\n' "$unit" >>"${SNAPSHOT_DIR}/disabled-units.txt"
}

disable_unit_if_present() {
  local unit="$1"
  # F3: "unit not found" is an acceptable no-op during the disable
  # phase — the unit was named in the script's worst-case list but
  # doesn't exist on this particular host. This is the ONLY phase
  # where unit-not-found is benign; pre-rename stop already handled
  # actually-running units fatally.
  if ! unit_exists "$unit"; then
    info "Skip (no such unit): $unit"
    return 0
  fi
  info "Disabling: $unit"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "systemctl disable --now $unit"
    record_disabled_unit "$unit"
    return 0
  fi
  if ! systemctl disable --now "$unit"; then
    # F3: critical sidecars (swtpm/virtiofsd/gpu) must not silently
    # fail to disable. By this phase the rename and verification have
    # already succeeded, so data is safe — but a phantom unit on next
    # boot could try to bind ports or open files we've moved.
    if is_critical_sidecar "$unit"; then
      err ""
      err "FATAL: failed to disable critical sidecar: $unit"
      err ""
      err "Current state:"
      err "  - State directories have been renamed AND verified byte-identical."
      err "  - TPM enrollment is preserved on disk."
      err "  - $unit is still enabled and will try to start on next boot,"
      err "    pointing at the OLD path that no longer exists."
      err ""
      err "What to do:"
      err "  1. Inspect:    systemctl status $unit"
      err "                 journalctl -xeu $unit --no-pager | tail -200"
      err "  2. Force-disable:"
      err "       systemctl disable $unit  ||  rm -f /etc/systemd/system/multi-user.target.wants/$unit"
      err "       systemctl daemon-reload"
      err "  3. Once disabled, run nixos-rebuild switch to land the new layout."
      err ""
      err "Do NOT run --rollback. The rename + verify succeeded; a rollback"
      err "would undo the safe-and-verified state to chase a disable failure."
      exit 1
    fi
    warn "disable failed for $unit (non-critical, continuing)"
  fi
  record_disabled_unit "$unit"
}

# F4: discover any nixling-sys-*-usbipd-* units that exist as static
# unit files on this host so we can disable them too. This is the
# disable-phase counterpart of stop_old_sidecars' dynamic discovery.
discover_new_usbipd_unit_files() {
  if ! command -v systemctl >/dev/null 2>&1; then
    return 0
  fi
  systemctl list-unit-files --no-legend --no-pager \
      'nixling-sys-*-usbipd-*.service' \
      'nixling-sys-*-usbipd-*.socket' \
      2>/dev/null \
    | awk '{print $1}' \
    | grep -E '^nixling-sys-.*-usbipd-.*\.(service|socket)$' \
    | LC_ALL=C sort -u || true
}

unit_disable_phase() {
  step "Unit-disable phase (old names that the new flake won't recreate)"

  local vm tmpl unit
  # Per-VM instance units (workload VMs only — net VMs handled separately).
  for vm in "${WORKLOAD_VMS[@]}"; do
    for tmpl in "${OLD_INSTANCE_UNITS_PER_VM[@]}"; do
      # tmpl is a hard-coded format string like "swtpm@%s.service" — use
      # bash substitution rather than printf to avoid SC2059 (variable
      # in printf format string).
      unit="${tmpl//%s/$vm}"
      disable_unit_if_present "$unit"
    done
  done

  # Net VMs: the old microvm@<env>-router.service must be disabled
  # because the new flake replaces it with microvm@sys-<env>-net.service.
  for unit in "${OLD_NET_VM_UNITS[@]}"; do
    disable_unit_if_present "$unit"
  done

  # USBIPD: old singleton + per-env proxies (pre- naming).
  for unit in "${OLD_USBIPD_UNITS[@]}"; do
    disable_unit_if_present "$unit"
  done

  # F4: new-naming USBIPD units (onwards) that may have been left
  # enabled by a partially-applied flake update.
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "enumerate nixling-sys-*-usbipd-* unit files and disable any present"
  else
    local nu
    while IFS= read -r nu; do
      [[ -z "$nu" ]] && continue
      disable_unit_if_present "$nu"
    done < <(discover_new_usbipd_unit_files)
  fi

  # The single-shot "vfsd-watchdog enable" units for the old VM names
  # are tied to instance names that may change. Disable the ones whose
  # instance name is being renamed (net VMs).
  for vm in "${NET_VMS_OLD[@]}"; do
    unit="nixling-vfsd-watchdog-${vm}-enable.service"
    disable_unit_if_present "$unit"
  done

  info "Unit-disable phase complete."
}

# -------------------------------------------------------------------
# Symlink cleanup
# -------------------------------------------------------------------

record_symlink_removed() {
  local link="$1" target="$2"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    dry "record removed symlink: $link -> $target"
    return 0
  fi
  printf '%s\t%s\n' "$link" "$target" >>"${SNAPSHOT_DIR}/removed-symlinks.tsv"
}

remove_back_compat_symlink() {
  local link="$1" expected_target="$2"
  if [[ ! -L "$link" ]]; then
    if [[ -e "$link" ]]; then
      warn "Path exists but is not a symlink, leaving alone: $link"
    fi
    return 0
  fi
  local actual
  actual="$(readlink "$link")"
  if [[ "$actual" != "$expected_target" ]]; then
    warn "Symlink target mismatch, leaving alone: $link -> $actual (expected $expected_target)"
    return 0
  fi
  info "Removing back-compat symlink: $link -> $actual"
  record_symlink_removed "$link" "$actual"
  run rm -f "$link"
}

symlink_cleanup_phase() {
  step "Back-compat symlink cleanup"
  remove_back_compat_symlink "/var/lib/microvms" "/var/lib/nixling"
  remove_back_compat_symlink "/var/lib/swtpm"   "/var/lib/nixling/swtpm"
}

# -------------------------------------------------------------------
# Forward run
# -------------------------------------------------------------------

forward_run() {
  # Early exit if already done. F6: read_marker_version() calls fail()
  # if the marker is present-but-corrupt, so a non-empty result here is
  # guaranteed to be a valid integer.
  local v
  v="$(read_marker_version)"
  if [[ -n "$v" ]]; then
    if (( v >= CURRENT_VERSION )); then
      info "Migration already applied (migrationVersion=$v >= $CURRENT_VERSION). Nothing to do."
      info "Marker: $MARKER_FILE"
      return 0
    fi
    # F6: a marker with a recognized-but-older version is not something
    # this script knows how to handle (no prior migration version
    # exists today). Fail loudly rather than silently re-run.
    fail "Marker file reports migrationVersion=$v, which is older than this script's CURRENT_VERSION=$CURRENT_VERSION.
This is unexpected — no prior migration version of this script has ever been shipped.

Current state: unclear. Either an older version of this script ran (none exists)
or the marker was hand-edited.

What to do:
  1. Inspect:                cat $MARKER_FILE
  2. Inspect state layout:   ls $STATE_DIR
  3. If pre-migration:       rm $MARKER_FILE && re-run this script.
  4. If post-migration:      hand-edit migrationVersion to $CURRENT_VERSION."
    return 0
  fi

  preflight
  # F1: stop sidecars + sync BEFORE snapshot. This closes the
  # swtpm-outlives-microvm race the script's own anomaly #1 documented.
  pre_rename_stop_phase
  snapshot_phase
  rename_phase
  verification_phase
  unit_disable_phase
  symlink_cleanup_phase

  step "Writing migration marker"
  write_marker "$SNAPSHOT_DIR"
  clear_in_progress

  print_summary
}

print_summary() {
  cat >&2 <<EOF

============================================================
Migration complete (dry-run=${DRY_RUN}).
============================================================

  Marker:    ${MARKER_FILE}
  Snapshot:  ${SNAPSHOT_DIR}
  Log:       ${LOG}

Next steps (you, manually):

  1. Update /etc/nixos to consume the new vicondoa/nixling flake
     (Phase 9 step 1, 4-8 in the plan), commit, then:

       sudo -A nixos-rebuild switch --flake /etc/nixos#desktop

  2. Verify the new unit names are in place (F11):

       nixling list
       # Should show entries like:
       #   nixling@work-aad      (workload)
       #   nixling@personal-dev  (workload)
       #   microvm@sys-work-net  (system)
       #   microvm@sys-personal-net  (system)
       # Old names (microvm@work-router, swtpm@work-aad standalone) must NOT appear.

       nixling status work-aad
       # Should report 'healthy' (or 'stopped' before you bring it up).
       # If it reports 'unknown unit' the rebuild didn't pick up the new flake.

  3. Bring VMs back up:

       nixling up work-aad
       nixling up personal-dev
       # net VMs autostart under their new names (sys-work-net etc.)

       # Then re-check:
       nixling status work-aad      # expect: running, healthy
       nixling status personal-dev  # expect: running, healthy

  4. Verify TPM enrollment survived. For each TPM-enabled VM
     (currently: ${TPM_VMS[*]}):

       systemctl status nixling-work-aad-swtpm
       # SSH into work-aad and run:
       tpm2_getcap properties-fixed

     Compare with the pre-migration capture at:
       ${SNAPSHOT_TPM_DIR}/work-aad.txt

     If 'tpm2_getcap' shows a freshly-initialised TPM (no platform
     hierarchy, default vendor strings, no Himmelblau credentials
     loaded after login), STOP. Run --rollback and investigate.

  5. After the new system is verified good (Entra-ID login still
     works without re-enrollment, no Intune device-tampering alert),
     the snapshot at ${SNAPSHOT_DIR} can be deleted:

       sudo rm -rf ${SNAPSHOT_DIR}

     Keep it for at least a week to allow rollback.

============================================================
EOF
}

# -------------------------------------------------------------------
# Rollback
# -------------------------------------------------------------------

rollback_renames() {
  local renames_file="${SNAPSHOT_DIR}/renames.tsv"
  if [[ ! -f "$renames_file" ]]; then
    info "No renames recorded; nothing to reverse."
    return 0
  fi
  # Reverse order so nested moves (swtpm inside vms/<vm>) are undone
  # before the outer dir is moved back.
  local src dst
  tac "$renames_file" | while IFS=$'\t' read -r src dst; do
    [[ -z "$src" || -z "$dst" ]] && continue
    if [[ -e "$src" && ! -e "$dst" ]]; then
      info "Already reversed: $dst -> $src"
      continue
    fi
    if [[ ! -e "$dst" && ! -e "$src" ]]; then
      warn "Reverse skip (both missing): $dst -> $src"
      continue
    fi
    if [[ -e "$dst" && -e "$src" ]]; then
      err "Cannot reverse: both $src and $dst exist."
      err ""
      err "Current state: a previous rollback attempt may have been interrupted,"
      err "or someone re-created the source directory while the destination was"
      err "still moved. The script will not merge directories."
      err ""
      err "What to do:"
      err "  1. Inspect both:"
      err "       ls -la $src"
      err "       ls -la $dst"
      err "  2. The snapshot path (with renames.tsv as the source of truth) is:"
      err "       $SNAPSHOT_DIR"
      err "  3. If the OLD-path ($src) is the authoritative copy, remove the"
      err "     new-path:    sudo -A rm -rf $dst"
      err "     If the NEW-path ($dst) is authoritative, remove the old:"
      err "                  sudo -A rm -rf $src"
      err "  4. Re-run --rollback."
      exit 1
    fi
    info "Reverse: $dst -> $src"
    local src_parent
    src_parent="$(dirname "$src")"
    run install -d -m 755 "$src_parent"
    if same_fs "$dst" "$src_parent"; then
      run mv -T "$dst" "$src"
    else
      run cp -a --reflink=auto -T "$dst" "$src"
      run rm -rf "$dst"
    fi
  done
}

rollback_units() {
  local file="${SNAPSHOT_DIR}/disabled-units.txt"
  if [[ ! -f "$file" ]]; then
    info "No units recorded as disabled; nothing to re-enable."
    return 0
  fi
  local unit
  while IFS= read -r unit; do
    [[ -z "$unit" ]] && continue
    if ! unit_exists "$unit"; then
      info "Skip re-enable (unit not present): $unit"
      continue
    fi
    info "Re-enable: $unit"
    run systemctl enable "$unit" || warn "enable failed for $unit"
  done <"$file"
}

rollback_symlinks() {
  local file="${SNAPSHOT_DIR}/removed-symlinks.tsv"
  if [[ ! -f "$file" ]]; then
    info "No symlinks recorded as removed; nothing to recreate."
    return 0
  fi
  local link target
  while IFS=$'\t' read -r link target; do
    [[ -z "$link" || -z "$target" ]] && continue
    if [[ -L "$link" || -e "$link" ]]; then
      info "Skip recreate (exists): $link"
      continue
    fi
    info "Recreate symlink: $link -> $target"
    run ln -s "$target" "$link"
  done <"$file"
}

rollback_run() {
  step "Rollback"
  check_tools

  # Find snapshot. Prefer in-progress (live partial migration); fall
  # back to marker (completed migration).
  local snap
  snap="$(read_in_progress_snapshot)"
  if [[ -z "$snap" ]]; then
    if [[ -f "$MARKER_FILE" ]]; then
      snap="$(sed -nE 's/.*"snapshotPath"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' "$MARKER_FILE" | head -1)"
    fi
  fi
  if [[ -z "$snap" ]]; then
    err "No snapshot reference found. Cannot --rollback automatically."
    err ""
    err "Current state: unknown — neither the in-progress marker nor the"
    err "completed-migration marker references a snapshot path."
    err "  checked: ${IN_PROGRESS_FILE}  (absent or no snapshotPath= line)"
    err "  checked: ${MARKER_FILE}        (absent or no snapshotPath JSON field)"
    err ""
    err "What to do:"
    err "  1. Find a snapshot dir under ${BACKUP_ROOT}/ — they're named"
    err "     by UTC timestamp (e.g. ${BACKUP_ROOT}/20250118T141500Z):"
    err "       ls -1t ${BACKUP_ROOT}/ 2>/dev/null"
    err "  2. Recreate the in-progress marker pointing at the most recent one:"
    err "       echo 'snapshotPath=${BACKUP_ROOT}/<ts>' > ${IN_PROGRESS_FILE}"
    err "  3. Re-run --rollback. If the snapshot still doesn't contain a"
    err "     renames.tsv or any hash files, the rollback has nothing to"
    err "     reverse and you should investigate the state manually."
    exit 1
  fi
  if [[ ! -d "$snap" ]]; then
    err "Snapshot dir missing: $snap"
    err ""
    err "Current state: a marker file references a snapshot directory that no"
    err "longer exists. Either the snapshot was deleted, or the marker is stale."
    err ""
    err "What to do:"
    err "  1. Check whether any snapshot remains:"
    err "       ls -1t ${BACKUP_ROOT}/"
    err "  2. If a usable snapshot exists, update the in-progress marker:"
    err "       echo 'snapshotPath=<path>' > ${IN_PROGRESS_FILE}"
    err "  3. If no snapshot remains and the system appears to be in the"
    err "     post-migration layout (state at $STATE_DIR/vms/), --rollback"
    err "     cannot run. The migration is effectively permanent without a"
    err "     backup."
    exit 1
  fi
  SNAPSHOT_DIR="$snap"
  SNAPSHOT_HASHES_DIR="$SNAPSHOT_DIR/hashes"
  SNAPSHOT_TPM_DIR="$SNAPSHOT_DIR/tpm2_getcap"
  info "Rolling back using snapshot: $SNAPSHOT_DIR"

  # Stop net VMs first if running — they may be holding the new state-dir
  # paths open. Workload VMs being up would have failed the forward run
  # too, but be defensive.
  local vm svc
  for vm in "${WORKLOAD_VMS[@]}" "${NET_VMS_OLD[@]}" "${NET_VMS_NEW[@]}"; do
    svc="microvm@${vm}.service"
    if ! check_unit_inactive "$svc"; then
      info "Stopping $svc before rollback"
      run systemctl stop "$svc" || warn "stop failed for $svc"
    fi
  done

  rollback_renames
  run sync
  rollback_units
  rollback_symlinks

  # F2: post-rollback verification must hash BOTH the public swtpm
  # state (under $STATE_DIR/swtpm/) AND the DynamicUser private state
  # (under $PRIVATE_DIR/swtpm/), matching the forward verification's
  # symmetry. Previously only the public path was rechecked, so a
  # corrupt private TPM dir would let rollback report "success".
  step "Post-rollback verification (public AND private TPM paths)"
  local ok=1 vm2 expect_pub expect_priv tmp_pub tmp_priv
  for vm2 in "${TPM_VMS[@]}"; do
    expect_pub="${SNAPSHOT_HASHES_DIR}/${vm2}__public.sha256"
    expect_priv="${SNAPSHOT_HASHES_DIR}/${vm2}__private.sha256"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      dry "verify ${STATE_DIR}/swtpm/${vm2}  against $expect_pub"
      dry "verify ${PRIVATE_DIR}/swtpm/${vm2} against $expect_priv"
      continue
    fi

    if [[ ! -f "$expect_pub" ]]; then
      err "Missing expected public hash file: $expect_pub"
      ok=0
      continue
    fi
    if [[ ! -f "$expect_priv" ]]; then
      err "Missing expected private hash file: $expect_priv"
      ok=0
      continue
    fi

    tmp_pub="$(mktemp -p "$SNAPSHOT_DIR" .rollback-pub.XXXXXX)"
    tmp_priv="$(mktemp -p "$SNAPSHOT_DIR" .rollback-priv.XXXXXX)"
    hash_dir "${STATE_DIR}/swtpm/${vm2}"   "$tmp_pub"
    hash_dir "${PRIVATE_DIR}/swtpm/${vm2}" "$tmp_priv"

    if ! diff -q "$expect_pub" "$tmp_pub" >/dev/null 2>&1; then
      err "Rollback hash mismatch: ${STATE_DIR}/swtpm/${vm2} (public)"
      diff -u "$expect_pub" "$tmp_pub" >&2 || true
      ok=0
    else
      info "Rollback hash OK: ${vm2} public  ($(wc -l <"$expect_pub") file(s))"
    fi
    if ! diff -q "$expect_priv" "$tmp_priv" >/dev/null 2>&1; then
      err "Rollback hash mismatch: ${PRIVATE_DIR}/swtpm/${vm2} (private)"
      diff -u "$expect_priv" "$tmp_priv" >&2 || true
      ok=0
    else
      info "Rollback hash OK: ${vm2} private ($(wc -l <"$expect_priv") file(s))"
    fi

    rm -f "$tmp_pub" "$tmp_priv"
  done

  if [[ "$ok" -ne 1 ]]; then
    err ""
    err "Rollback completed but TPM hashes do NOT match the pre-migration snapshot."
    err ""
    err "Current state: state dirs are at the OLD paths but their byte content"
    err "differs from the snapshot. The TPM enrollment may be corrupted."
    err ""
    err "The snapshot at $SNAPSHOT_DIR is intact. Do NOT:"
    err "  - boot the work-aad VM"
    err "  - run nixos-rebuild switch"
    err "  - delete the snapshot dir"
    err ""
    err "Recovery options:"
    err "  1. Compare hashes manually to identify which file diverged:"
    err "       diff <(sort $SNAPSHOT_HASHES_DIR/work-aad__public.sha256) \\"
    err "            <(cd $STATE_DIR/swtpm/work-aad && find . -type f -print0 | \\"
    err "                  LC_ALL=C sort -z | xargs -0 sha256sum | sort)"
    err "  2. Restore from off-host backup if you have one"
    err "  3. Accept re-enrollment as a last resort (this triggers Intune"
    err "     device-tampering alerts — coordinate with your IT/Security team)."
    exit 1
  fi

  # Drop the marker — we're back to pre-migration state.
  if [[ -f "$MARKER_FILE" ]]; then
    info "Removing migration marker: $MARKER_FILE"
    run rm -f "$MARKER_FILE"
  fi
  clear_in_progress

  cat >&2 <<EOF

============================================================
Rollback complete.
============================================================

State directories are back to pre-migration paths.
Disabled units have been re-enabled.
Back-compat symlinks have been recreated.

Snapshot retained at: ${SNAPSHOT_DIR}
(safe to delete once you've confirmed the system boots and VMs work)

Next steps:
  1. Do NOT run nixos-rebuild against the new flake yet.
  2. If /etc/nixos was already partially updated to consume the new
     flake, revert those commits before rebuilding:
       cd /etc/nixos && git log --oneline | head
       git revert <commit>
  3. Run \`sudo -A nixos-rebuild switch --flake /etc/nixos#desktop\`
     to restore the old activation.
  4. Bring VMs back up the old way:
       nixling up work-aad
       nixling up personal-dev
     (net VMs autostart under their old names)

============================================================
EOF
}

# -------------------------------------------------------------------
# Main
# -------------------------------------------------------------------

main() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    info "=== DRY RUN MODE — no changes will be made ==="
  fi

  # Root check must run before acquire_lock; the lock file lives under
  # /var/lib/nixling which non-root users can't create.
  check_root

  acquire_lock

  if [[ "$ROLLBACK" -eq 1 ]]; then
    rollback_run
  else
    forward_run
  fi
}

main "$@"
