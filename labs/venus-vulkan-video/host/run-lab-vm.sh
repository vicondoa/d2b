#!/usr/bin/env bash
# run-lab-vm.sh - launch the Venus Vulkan Video lab VM.
#
# Everything runs as the invoking user, from nix build outputs. There is NO
# nixos-rebuild switch, NO /etc/nixos edit, NO systemd unit, and NO involvement
# of d2bd or the privileged broker. See ../AGENTS.md.
#
# Process topology (started in this order):
#
#   passt --vhost-user      unprivileged NAT. No TAP, no CAP_NET_ADMIN, no host
#                           routing change. CH connects as the vhost-user client.
#   cage                    per-run NESTED compositor. crosvm is pointed at THIS
#                           socket, never the operator's real one, so a
#                           guest->renderer compromise cannot reach the real
#                           desktop session.
#   bwrap + crosvm device gpu
#                           GPU sidecar, linked against the LAB virglrenderer,
#                           confined to the bind set in AGENTS.md rule 6.
#   cloud-hypervisor        the VM itself.
#
# All four are torn down in reverse order on EXIT/INT/TERM, and the /dev/kvm
# grant is revoked, so a crashed run does not leave sockets, GPU fds or an
# open KVM grant behind.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)

# The grant helper is normally a sibling in the repo. When this script runs
# from the Nix store (via `nix run .#lab-vm`) it is copied there on its own,
# so the sibling does not exist; the app passes the store path explicitly.
GRANT_KVM="${VENUS_LAB_GRANT_KVM:-$HERE/grant-kvm.sh}"

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab"
RUN_ID="$$-$(date +%s)"
RUN_DIR="${XDG_RUNTIME_DIR:-/tmp}/venus-lab/$RUN_ID"

MEM="${VENUS_LAB_MEM:-8G}"
CPUS="${VENUS_LAB_CPUS:-4}"
DISK_SIZE="${VENUS_LAB_DISK:-16G}"

# Host loopback port forwarded to the guest's sshd. Overridable so two lab runs
# (or a lab run alongside something else on 2222) do not collide.
SSH_PORT="${VENUS_LAB_SSH_PORT:-2222}"

# Populated as children start, torn down in reverse.
declare -a CHILD_PIDS=()
KVM_GRANTED=0

log()  { printf '[lab] %s\n' "$*" >&2; }
die()  { printf '[lab] ERROR: %s\n' "$*" >&2; exit 1; }

cleanup() {
  local rc=$?
  log "tearing down (rc=$rc)"
  # Reverse order: VM first, then sidecars, so the guest sees a clean stop
  # before its devices vanish.
  #
  # Kill the process GROUP, not just the direct child. crosvm runs under
  # bwrap, so the pid recorded here is bwrap's; signalling only that leaves
  # crosvm orphaned, still holding the GPU fd and its socket. Observed in
  # practice, hence setsid + negative-pid signalling below.
  local i pid
  for (( i=${#CHILD_PIDS[@]}-1; i>=0; i-- )); do
    pid="${CHILD_PIDS[$i]}"
    kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
  done
  sleep 1
  for (( i=${#CHILD_PIDS[@]}-1; i>=0; i-- )); do
    pid="${CHILD_PIDS[$i]}"
    kill -9 -- "-$pid" 2>/dev/null || kill -9 "$pid" 2>/dev/null || true
  done

  # Revoke ANY per-user /dev/kvm ACL entry, not only one this launcher created.
  # Revoking only our own grant left a manual grant (which the README used to
  # suggest) or a grant orphaned by an earlier crash in place across a clean
  # exit -- silently widening AE-1 for the rest of the session. The launcher
  # now owns the grant lifecycle unconditionally.
  if bash "$GRANT_KVM" --has-acl >/dev/null 2>&1; then
    if [ "$KVM_GRANTED" = "1" ]; then
      log "revoking /dev/kvm grant (created by this run)"
    else
      log "revoking pre-existing /dev/kvm ACL entry (not created by this run)"
    fi
    bash "$GRANT_KVM" --revoke || true
  fi

  [ -d "$RUN_DIR" ] && rm -rf "$RUN_DIR"
  log "done"
}
trap cleanup EXIT INT TERM

need() { command -v "$1" >/dev/null 2>&1 || die "missing '$1' on PATH"; }

# --- preflight --------------------------------------------------------------
preflight() {
  log "preflight"

  for b in passt cage bwrap cloud-hypervisor qemu-img; do need "$b"; done

  # Contention warning, not a hard failure: the lab shares /dev/kvm, the render
  # node, /dev/udmabuf, RAM and the GPU with any running d2b VMs. Measurements
  # taken alongside live VMs are not trustworthy.
  #
  # Deliberately does NOT shell out to the d2b CLI: the isolation contract
  # (AGENTS.md rule 3) forbids touching the d2b control plane, and an earlier
  # version of this preflight violated that by calling `d2b vm list`. Counting
  # hypervisor processes needs no control-plane access and catches non-d2b VMs
  # competing for the same GPU too.
  local vm_procs
  vm_procs=$(pgrep -c -f 'cloud-hypervisor|crosvm device gpu|qemu-system' 2>/dev/null || true)
  if [ "${vm_procs:-0}" -gt 0 ]; then
    log "WARNING: ${vm_procs} VM/GPU-sidecar process(es) already running."
    log "         They share /dev/kvm, the GPU, /dev/udmabuf and RAM with this lab."
    log "         Stop them before taking any benchmark numbers."
  fi

  local free_gb
  free_gb=$(free -g 2>/dev/null | awk '/^Mem:/ {print $7}' || echo 99)
  if [ "${free_gb:-99}" -lt 8 ]; then
    log "WARNING: only ${free_gb}G RAM available; requested $MEM for the guest"
  fi

  # /dev/kvm is the single privilege gap (AE-1). Grant only if needed, and
  # remember to revoke -- cleanup() does that on every exit path it can see.
  if ! bash "$GRANT_KVM" --status >/dev/null 2>&1; then
    log "/dev/kvm not accessible; requesting a reversible grant (needs sudo)"
    bash "$GRANT_KVM" --grant >/dev/null || die "could not obtain /dev/kvm access"
    KVM_GRANTED=1
  fi

  mkdir -p "$RUN_DIR" "$STATE_DIR"
}

# --- writable disk ----------------------------------------------------------
# make-disk-image output lives read-only in the Nix store, so the guest needs a
# private writable copy for boot state and the Firefox profile.
#
# The copy is stamped with the store path it came from. Without that stamp a
# rebuilt image silently keeps the previous disk while the launcher passes the
# NEW kernel/initrd/init, so the initrd looks for a closure that is not on the
# disk. That fails as `Failed to start Find NixOS closure` and drops to
# emergency mode -- a message that names neither the disk nor the mismatch, and
# reads like initrd corruption. Detect it instead of leaving it to be
# rediscovered.
prepare_disk() {
  local src="$1" dst="$STATE_DIR/lab-disk.raw" stamp="$STATE_DIR/lab-disk.src"
  local have=""
  [ -f "$stamp" ] && have=$(cat "$stamp")

  local reason=""
  if [ ! -f "$dst" ]; then
    reason="no disk yet"
  elif [ "${VENUS_LAB_RESET_DISK:-0}" = "1" ]; then
    reason="VENUS_LAB_RESET_DISK=1"
  elif [ "$have" != "$src" ]; then
    # Keeping the stale disk here would boot a kernel against a closure that
    # does not exist on it, so this is a correctness reset, not a convenience.
    reason="guest image changed"
    log "guest image changed:"
    log "  disk was built from: ${have:-<unstamped>}"
    log "  launcher now passes: $src"
  fi

  if [ -n "$reason" ]; then
    log "materializing writable disk at $dst ($reason)"
    rm -f "$dst" "$stamp"
    install -m 0600 /dev/null "$dst"
    cat "$src" > "$dst"
    qemu-img resize -f raw "$dst" "$DISK_SIZE" >/dev/null
    printf '%s' "$src" > "$stamp"
  else
    log "reusing existing writable disk $dst (VENUS_LAB_RESET_DISK=1 to recreate)"
  fi
  printf '%s' "$dst"
}

wait_for_socket() {
  local path="$1" what="$2" tries=100
  while [ "$tries" -gt 0 ]; do
    [ -S "$path" ] && return 0
    sleep 0.1
    tries=$((tries - 1))
  done
  die "$what socket never appeared at $path"
}

main() {
  local image="${VENUS_LAB_IMAGE:-}" kernel="${VENUS_LAB_KERNEL:-}" initrd="${VENUS_LAB_INITRD:-}"
  [ -n "$image" ]  || die "set VENUS_LAB_IMAGE to the built guest image (nix build .#guestImage)"
  [ -n "$kernel" ] || die "set VENUS_LAB_KERNEL to the guest kernel (nix build .#guestKernel)"

  preflight

  local disk; disk=$(prepare_disk "$image")

  # 1. unprivileged networking
  #
  # -t forwards a HOST LOOPBACK port into the guest, which is what makes the
  # guest scriptable. Without it every guest observation needs a predefined
  # systemd unit and therefore a full image rebuild -- one rebuild per
  # experiment. Bound to 127.0.0.1 explicitly so the lab guest is never
  # reachable from the network.
  log "starting passt (vhost-user, ssh 127.0.0.1:$SSH_PORT -> guest 22)"
  setsid passt --vhost-user --socket "$RUN_DIR/passt.sock" --foreground \
    -t "127.0.0.1/$SSH_PORT:22" &
  CHILD_PIDS+=($!)
  wait_for_socket "$RUN_DIR/passt.sock" "passt"

  # 2. nested compositor -- crosvm must NEVER see the real Wayland socket.
  #
  # wlroots compositors take the next free wayland-N, so the name cannot be
  # assumed: this host already has wayland-0..2 in use. Snapshot the existing
  # sockets, start cage, then poll for the one that appeared. Guessing a name
  # here would silently hand the sidecar the operator's real session.
  log "starting nested compositor (cage)"
  local wl_dir="${XDG_RUNTIME_DIR:-/tmp}"

  # List wayland-N sockets (excluding .lock files) without parsing `ls`.
  list_wl_sockets() {
    local s
    for s in "$wl_dir"/wayland-*; do
      [ -S "$s" ] && printf '%s\n' "$s"
    done | sort
  }

  local before after nested_sock=""
  before=$(list_wl_sockets || true)
  log "existing wayland sockets: $(printf '%s' "$before" | tr '\n' ' ')"

  setsid cage -- sleep infinity > "$RUN_DIR/cage.log" 2>&1 &
  CHILD_PIDS+=($!)

  local tries=150
  while [ "$tries" -gt 0 ]; do
    after=$(list_wl_sockets || true)
    nested_sock=$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") 2>/dev/null | head -1 || true)
    if [ -n "$nested_sock" ] && [ -S "$nested_sock" ]; then
      break
    fi
    nested_sock=""
    sleep 0.1
    tries=$((tries - 1))
  done
  if [ -z "$nested_sock" ]; then
    log "cage did not create a new Wayland socket; its log follows:"
    sed 's/^/  cage| /' "$RUN_DIR/cage.log" >&2 || true
    log "sockets now: $(list_wl_sockets | tr '\n' ' ')"
    die "cage never created a nested Wayland socket"
  fi

  case "$nested_sock" in
    "$wl_dir"/wayland-*) ;;
    *) die "refusing to use unexpected socket path: $nested_sock" ;;
  esac
  log "nested compositor socket: $nested_sock"

  # 3. GPU sidecar, confined.
  #
  # Mount isolation alone is not enough once W1-W3 add a guest-controlled
  # parser to this process. Namespaces and environment are also constrained:
  #   --unshare-net   the sidecar speaks only AF_UNIX (vhost-user + Wayland);
  #                   it has no legitimate need for host networking.
  #   --unshare-pid   with a fresh --proc, so it cannot see or signal host
  #                   processes. --die-with-parent ties it to the launcher.
  #   --unshare-ipc/uts  no shared SysV IPC or hostname surface.
  #   --clearenv      the operator's full environment is not handed to a
  #                   process that will parse untrusted input; only the
  #                   variables crosvm/virglrenderer actually need are passed.
  log "starting crosvm GPU sidecar (lab virglrenderer, bwrap-confined)"
  local bw=(
    --ro-bind /nix/store /nix/store
    --ro-bind /run/opengl-driver /run/opengl-driver
    --ro-bind /etc /etc
    --ro-bind /sys /sys
    --proc /proc --tmpfs /tmp --dev /dev
    --bind "$RUN_DIR" "$RUN_DIR"
    --unshare-net --unshare-pid --unshare-ipc --unshare-uts
    --die-with-parent --new-session
    --clearenv
    --setenv PATH "$PATH"
    --setenv XDG_RUNTIME_DIR "${XDG_RUNTIME_DIR:-/tmp}"
    --setenv HOME "$RUN_DIR"
  )
  # Diagnostic passthrough. --clearenv means nothing reaches the sidecar unless
  # it is named here, which is the right default but also means a debug knob is
  # silently ignored rather than refused -- so these are explicit.
  #
  # VKR_DEBUG=validate forces the Vulkan validation layer inside the renderer,
  # which is the only way to see WHY a Venus submission failed: rutabaga
  # reports it to the guest as a bare ComponentError(22), i.e. EINVAL, with no
  # detail at all.
  #
  # VREND_DEBUG=blit serves the same purpose for the GL side. A failing blit
  # surfaces only as "failed to dispatch BLIT: 22", and that 22 is synthesised:
  # vrend_decode_blit returns 0, then the dispatcher turns any pending GL error
  # into EINVAL. The real error is whatever glBlitFramebuffer raised, and the
  # blit debug path is what prints the formats and regions behind it.
  local ev
  for ev in VKR_DEBUG VN_DEBUG VN_PERF VIRGL_LOG_LEVEL MESA_LOG_LEVEL \
            VREND_DEBUG VIRGL_TRACE_DMABUF_IMPORT VIRGL_TRACE_BLIT VIRGL_TRACE_IMPORT VK_LOADER_DEBUG VK_INSTANCE_LAYERS; do
    [ -n "${!ev:-}" ] && bw+=(--setenv "$ev" "${!ev}")
  done
  # The broad /etc bind is needed for the Vulkan loader and NSS config, but it
  # would also expose /etc/d2b to the sidecar -- which the lab isolation
  # contract (AGENTS.md rule 3) forbids. Mask it with an empty tmpfs so the
  # d2b control-plane state is provably not visible inside the sandbox.
  [ -e /etc/d2b ] && bw+=(--tmpfs /etc/d2b)
  [ -d /run/current-system/sw ] && bw+=(--ro-bind /run/current-system/sw /run/current-system/sw)
  [ -S "$nested_sock" ] && bw+=(--bind "$nested_sock" "$nested_sock")
  local d
  for d in /dev/dri/renderD128 /dev/nvidia0 /dev/nvidiactl /dev/nvidia-modeset \
           /dev/nvidia-uvm /dev/nvidia-uvm-tools /dev/udmabuf; do
    [ -e "$d" ] && bw+=(--dev-bind "$d" "$d")
  done

  setsid bwrap "${bw[@]}" -- crosvm device gpu \
      --socket "$RUN_DIR/gpu.sock" \
      --wayland-sock "$nested_sock" \
      --gpu-device-node /dev/dri/renderD128 \
      --params '{"context-types":"virgl2:venus","implicit-render-server":true,"external-blob":true}' &
  CHILD_PIDS+=($!)
  wait_for_socket "$RUN_DIR/gpu.sock" "crosvm gpu"

  # 4. the VM.
  #   shared=true is REQUIRED: CH rejects vhost-user without shared memory.
  #   vhost_mode=client is explicit -- relying on the default is a silent
  #   networking failure if it ever changes.
  # 4. the VM.
  #   shared=true is REQUIRED: CH rejects vhost-user without shared memory.
  #   vhost_mode=client is explicit -- relying on the default is a silent
  #   networking failure if it ever changes.
  #
  # Serial goes to a file when VENUS_LAB_SERIAL_LOG is set, so the boot can be
  # validated non-interactively; otherwise it attaches to the terminal.
  local serial_arg="tty"
  if [ -n "${VENUS_LAB_SERIAL_LOG:-}" ]; then
    serial_arg="file=$VENUS_LAB_SERIAL_LOG"
    log "serial console -> $VENUS_LAB_SERIAL_LOG"
  fi

  log "starting cloud-hypervisor"
  cloud-hypervisor \
    --cpus "boot=$CPUS" \
    --memory "size=$MEM,shared=true" \
    --kernel "$kernel" \
    ${initrd:+--initramfs "$initrd"} \
    --cmdline "console=ttyS0 root=/dev/vda rw ${VENUS_LAB_INIT:+init=$VENUS_LAB_INIT} loglevel=4" \
    --disk "path=$disk,image_type=raw,readonly=off" \
    --net "vhost_user=true,socket=$RUN_DIR/passt.sock,vhost_mode=client,mac=52:54:00:12:34:56" \
    --gpu "socket=$RUN_DIR/gpu.sock" \
    --api-socket "$RUN_DIR/ch.sock" \
    --serial "$serial_arg" --console off &
  CHILD_PIDS+=($!)

  log "VM running. run dir: $RUN_DIR"
  log "Ctrl-C to stop; everything is torn down and the KVM grant revoked."
  wait "${CHILD_PIDS[-1]}"
}

main "$@"
