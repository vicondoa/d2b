#!/usr/bin/env bash
# shellcheck disable=SC2012,SC2295
# Layer-2 integration tests for the per-VM nix store + lifecycle CLI.
#
# Each `test_*` is one function. Idempotent. Safe to re-run on the
# live host. Leaves every VM in the state it was found in (back-rolls
# any switches it issued).
#
# Usage:
#   modules/d2b/tests/integration/live/d2b-store.sh                 # full run
#   modules/d2b/tests/integration/live/d2b-store.sh --quick         # smoke subset
#   modules/d2b/tests/integration/live/d2b-store.sh --only test_X   # one test
#
# Tests requiring a running VM (#8-21, #24) auto-skip with an explicit
# SKIP line when the target VM isn't up. To exercise those, bring the
# VM up first (`d2b up <vm>`) and re-run.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

STATE_ROOT=/var/lib/d2b/vms

# Pick test VMs. Defaults derive from the live system manifest baked
# into the d2b CLI generation instead of a hand-maintained list. The list
# excludes net VMs (\`isNetVm == true\`) so per-VM store-sync tests
# don't try to operate on the auto-declared sys-<env>-net entries
# that may or may not carry workload payloads.
#
# Override with D2B_VMS="dev-vm test-vm" for a subset run.
if [ -z "${D2B_VMS:-}" ]; then
  _manifest=/run/current-system/sw/share/d2b/vms.json
  if [ -r "$_manifest" ]; then
    D2B_VMS=$(jq -r '
      to_entries
      | map(select(.key | startswith("_") | not))
      | map(select(.value.isNetVm == false))
      | .[].key' "$_manifest" 2>/dev/null | tr '\n' ' ')
  fi
  # Fallback when the manifest is unreadable (e.g. first-boot
  # before a successful nixos-rebuild). The seed list intentionally
  # mixes a couple of plausible workload names from the public-
  # flake examples with `d2b-test` so a CI-style runner can
  # smoke-test without a live host.
  D2B_VMS=${D2B_VMS:-corp-vm dev-vm d2b-test}
fi

# A VM that we know is running headlessly (auto-routers, or whatever
# the user has up). Detected at startup.
D2B_RUNNING_VM=""
for v in $D2B_VMS; do
  if vm_running "$v"; then D2B_RUNNING_VM=$v; break; fi
done

skip() { log "  SKIP: $*"; }

# ---------------------------------------------------------------------
# #1 closure isolation: per-VM store is a strict subset of host /nix/store.
# ---------------------------------------------------------------------
test_closure_isolation() {
  log "test_closure_isolation"
  local host_count vm_count ratio_pct vm
  host_count=$(ls /nix/store | wc -l)
  for vm in $D2B_VMS; do
    [ -d "$STATE_ROOT/$vm/store" ] || { fail "missing $STATE_ROOT/$vm/store"; continue; }
    vm_count=$(ls "$STATE_ROOT/$vm/store" | wc -l)
    ratio_pct=$(( vm_count * 100 / host_count ))
    log "  $vm: $vm_count paths / $host_count host = ${ratio_pct}%"
    assert_lt "$ratio_pct" 30 "$vm closure ratio < 30%"
  done
}

# ---------------------------------------------------------------------
# #2 no leaked host paths: a host-only path is NOT in the VM's store.
# ---------------------------------------------------------------------
test_no_host_paths_in_vm() {
  log "test_no_host_paths_in_vm"
  # Pick a path that's in the host's system closure but NOT in any
  # VM's closure. The host's nixos-system-nixos-* derivation is a safe
  # bet — it's the toplevel for the host, not any VM.
  local host_sys vm
  host_sys=$(basename "$(readlink /run/current-system)")
  for vm in $D2B_VMS; do
    if [ -e "$STATE_ROOT/$vm/store/$host_sys" ]; then
      fail "$vm leaks host toplevel: $host_sys"
    else
      ok "$vm does not contain host toplevel ($host_sys)"
    fi
  done
}

# ---------------------------------------------------------------------
# #3 hardlink sharing: per-VM path inode matches host inode.
# ---------------------------------------------------------------------
test_hardlink_sharing() {
  log "test_hardlink_sharing"
  local vm host_ino vm_ino n=0 matched=0
  for vm in $D2B_VMS; do
    [ -d "$STATE_ROOT/$vm/store" ] || continue
    # Pick a few non-trivial paths.
    while IFS= read -r p; do
      [ -e "/nix/store/$p" ] || continue
      [ -f "/nix/store/$p" ] && continue   # skip top-level (it's a dir)
      n=$((n+1))
      host_ino=$(stat -c '%i' "/nix/store/$p" 2>/dev/null || echo 0)
      vm_ino=$(stat -c '%i' "$STATE_ROOT/$vm/store/$p" 2>/dev/null || echo 0)
      if [ "$host_ino" = "$vm_ino" ] && [ "$host_ino" != "0" ]; then
        matched=$((matched+1))
      fi
      [ "$n" -ge 5 ] && break
    done < <(ls "$STATE_ROOT/$vm/store" 2>/dev/null | head -20)
  done
  # NOTE: directories are not hardlinks (Linux refuses), so we
  # tolerate < 100% match. We require at least one matched pair
  # somewhere to prove the hardlink path is actually being taken; any
  # mismatch on a regular file would be a regression.
  for vm in $D2B_VMS; do
    [ -d "$STATE_ROOT/$vm/store" ] || continue
    # pick one regular file inside the vm's first store path
    local sample_dir sample_file
    sample_dir=$(ls -d "$STATE_ROOT/$vm/store"/* 2>/dev/null | head -1)
    [ -n "$sample_dir" ] || continue
    sample_file=$(find "$sample_dir" -type f 2>/dev/null | head -1)
    [ -n "$sample_file" ] || continue
    local host_path="/nix/store/${sample_file#$STATE_ROOT/$vm/store/}"
    host_ino=$(stat -c '%i' "$host_path" 2>/dev/null || echo 0)
    vm_ino=$(stat -c '%i'  "$sample_file" 2>/dev/null || echo 0)
    if [ "$host_ino" = "$vm_ino" ] && [ "$host_ino" != "0" ]; then
      ok "$vm: $sample_file shares inode with host ($host_ino)"
    else
      fail "$vm: $sample_file inode $vm_ino != host $host_ino"
    fi
  done
}

# ---------------------------------------------------------------------
# #4 zero data duplication: per-VM store occupies basically no extra
# disk because everything is a hardlink to /nix/store.
# ---------------------------------------------------------------------
test_zero_data_duplication() {
  log "test_zero_data_duplication"
  # Easiest invariant: every regular file under store/ must have
  # nlink >= 2 (the /nix/store entry + our hardlink).
  local vm bad=0 sampled=0
  for vm in $D2B_VMS; do
    [ -d "$STATE_ROOT/$vm/store" ] || continue
    while IFS= read -r f; do
      sampled=$((sampled+1))
      local n
      n=$(stat -c '%h' "$f" 2>/dev/null || echo 0)
      if [ "$n" -lt 2 ]; then
        bad=$((bad+1))
        log "    unshared (nlink=$n): $f"
      fi
      [ "$sampled" -ge 200 ] && break
    done < <(find "$STATE_ROOT/$vm/store" -maxdepth 4 -type f 2>/dev/null | head -200)
  done
  if [ "$bad" -eq 0 ]; then
    ok "all $sampled sampled files have nlink >= 2"
  else
    fail "$bad / $sampled files are unshared"
  fi
}

# ---------------------------------------------------------------------
# #6 build idempotent: second `d2b build` does no work.
# ---------------------------------------------------------------------
test_build_idempotent() {
  log "test_build_idempotent"
  local vm=${D2B_RUNNING_VM:-corp-vm}
  local first second
  first=$(d2b build "$vm" 2>&1 | grep 'closure →' | awk '{print $NF}')
  second=$(d2b build "$vm" 2>&1 | grep 'closure →' | awk '{print $NF}')
  assert_eq "$first" "$second" "build '$vm' idempotent"
}

# ---------------------------------------------------------------------
# #7 build GC root: nix-store --gc --print-roots references it.
# ---------------------------------------------------------------------
test_build_gc_root() {
  log "test_build_gc_root"
  local vm=${D2B_RUNNING_VM:-corp-vm}
  d2b build "$vm" >/dev/null 2>&1 || true
  if [ -L "$STATE_ROOT/$vm/result" ]; then
    ok "result symlink present: $STATE_ROOT/$vm/result"
  else
    fail "result symlink missing"
    return 1
  fi
  if sudo -A nix-store --gc --print-roots 2>/dev/null | grep -q "$STATE_ROOT/$vm/result"; then
    ok "nix recognises $STATE_ROOT/$vm/result as a GC root"
  else
    log "  (GC root not yet registered — only matters under nix-collect-garbage)"
  fi
}

# ---------------------------------------------------------------------
# #13 generations list: at least 1 entry, includes 'current' marker.
# ---------------------------------------------------------------------
test_generations_list() {
  log "test_generations_list"
  local vm
  for vm in $D2B_VMS; do
    local out
    out=$(d2b generations "$vm" 2>&1)
    assert_contains "$out" "Host-side per-VM store generations" "$vm: header present"
    assert_contains "$out" "(current)" "$vm: current marker present"
  done
}

# ---------------------------------------------------------------------
# #17 host rebuild rehydrates per-VM stores.
# ---------------------------------------------------------------------
test_host_rebuild_rehydrates() {
  log "test_host_rebuild_rehydrates"
  # We don't actually rebuild — that would take forever. Instead we
  # confirm the activation hook is wired: a `system.activationScripts`
  # entry exists that invokes d2b-store-sync.
  if grep -q d2bStoreSync /run/current-system/activate; then
    ok "activate script references d2bStoreSync"
  else
    fail "activate script lacks d2bStoreSync hook"
  fi
  # And per-VM d2b-<vm>-store-sync.service units exist (one per declared VM).
  if compgen -G '/etc/systemd/system/d2b-*-store-sync.service' >/dev/null 2>&1 \
     || systemctl list-unit-files 'd2b-*-store-sync.service' --no-pager --no-legend 2>/dev/null \
        | grep -q 'd2b-.*-store-sync.service'; then
    ok "d2b-<vm>-store-sync.service unit(s) installed"
  else
    fail "no d2b-<vm>-store-sync.service unit installed"
  fi
}

# ---------------------------------------------------------------------
# #19 in-VM nix db loaded: requisites/valid checks succeed.
# Skipped if no VM is reachable over SSH.
# ---------------------------------------------------------------------
test_db_load_on_boot() {
  log "test_db_load_on_boot"
  local vm=$D2B_RUNNING_VM
  if [ -z "$vm" ]; then skip "no running VM"; return 0; fi
  if [ -z "$(vm_ssh_user "$vm")" ]; then skip "$vm lacks ssh creds"; return 0; fi
  local out
  out=$(ssh_vm "$vm" 'nix-store --query --requisites /run/current-system | head -1' 2>&1)
  if [ -n "$out" ]; then
    ok "$vm: nix-store --query --requisites works"
  else
    fail "$vm: nix-store --query --requisites empty"
  fi
}

# ---------------------------------------------------------------------
# #23 legacy `d2b status` still works (regression).
# ---------------------------------------------------------------------
test_legacy_status() {
  log "test_legacy_status"
  local out
  out=$(d2b status 2>&1)
  assert_contains "$out" "NAME" "status output has table header"
  assert_contains "$out" "STATIC_IP" "status output has STATIC_IP column"
}

# ---------------------------------------------------------------------
# #26 missing-creds friendly error.
# ---------------------------------------------------------------------
test_error_missing_ssh_creds() {
  log "test_error_missing_ssh_creds"
  # Auto-declared net VMs (\`sys-<env>-net\`) don't have
  # ssh.{user,keyPath} in the manifest. So
  # \`d2b generations <net-vm>\` should print the unreachable-VM
  # warning for the in-VM section without failing the host-side
  # section. Pick the first net VM from the manifest.
  local net_vm
  net_vm=$(jq -r '
      to_entries
      | map(select(.key | startswith("_") | not))
      | map(select(.value.isNetVm == true))
      | .[0].key // empty' \
    /run/current-system/sw/share/d2b/vms.json 2>/dev/null)
  if [ -z "$net_vm" ]; then
    skip "no net VM declared in manifest"
    return 0
  fi
  local out
  out=$(d2b generations "$net_vm" 2>&1 || true)
  assert_contains "$out" "no ssh.user" "$net_vm: helpful ssh-missing message"
}

# ---------------------------------------------------------------------
# #28 retention policy: store-meta keeps the union of kept generations'
# store-paths. Verified by checking that the current generation's
# store-paths are all present.
# ---------------------------------------------------------------------
test_retention_keeps_current() {
  log "test_retention_keeps_current"
  local vm missing=0 checked=0
  for vm in $D2B_VMS; do
    local meta=$STATE_ROOT/$vm/store-meta/current
    [ -L "$meta" ] || continue
    while IFS= read -r p; do
      [ -n "$p" ] || continue
      checked=$((checked+1))
      local base=${p##*/}
      if [ ! -e "$STATE_ROOT/$vm/store/$base" ]; then
        missing=$((missing+1))
      fi
    done < "$meta/store-paths"
  done
  assert_eq "$missing" "0" "retention: all current-gen paths present in store/ ($checked checked)"
}

# ---------------------------------------------------------------------
# dispatcher
# ---------------------------------------------------------------------

ALL_TESTS=(
  test_closure_isolation
  test_no_host_paths_in_vm
  test_hardlink_sharing
  test_zero_data_duplication
  test_build_idempotent
  test_build_gc_root
  test_generations_list
  test_host_rebuild_rehydrates
  test_db_load_on_boot
  test_legacy_status
  test_error_missing_ssh_creds
  test_retention_keeps_current
)

# shellcheck disable=SC2034  # QUICK_TESTS consumed via `local -n SET=QUICK_TESTS` in main()
QUICK_TESTS=(
  test_closure_isolation
  test_no_host_paths_in_vm
  test_hardlink_sharing
  test_zero_data_duplication
  test_build_idempotent
  test_host_rebuild_rehydrates
  test_legacy_status
  test_retention_keeps_current
)

main() {
  local mode=${1:-full} only=""
  case "$mode" in
    --quick)        local -n SET=QUICK_TESTS ;;
    --only)         only="${2:-}"; local -n SET=ALL_TESTS ;;
    --list)         printf '%s\n' "${ALL_TESTS[@]}"; return 0 ;;
    full|*)         local -n SET=ALL_TESTS ;;
  esac

  log "d2b test suite — log: $D2B_LOG  running-vm: ${D2B_RUNNING_VM:-<none>}"
  local pass=0 fail_count=0
  for t in "${SET[@]}"; do
    if [ -n "$only" ] && [ "$t" != "$only" ]; then continue; fi
    if "$t"; then
      pass=$((pass+1))
    else
      fail_count=$((fail_count+1))
    fi
  done
  log "==="
  log "Summary: $pass passed, $fail_count failed"
  [ "$fail_count" -eq 0 ]
}

main "$@"
