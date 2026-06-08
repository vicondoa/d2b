# The 'nixling' CLI: a single command with subcommands for managing
# every microVM declared via the nixling framework.
#
# Generates a JSON manifest at Nix-eval time mapping each VM to its
# per-VM metadata (capabilities, sockets, IPs, etc). The shell uses
# 'jq' to look up that metadata at runtime.
#
# Subcommands:
#   nixling list                  -- enumerate declared VMs + capabilities
#   nixling up    <vm> [-d|--detach]   -- bring up (interactive for graphics
#                                          VMs; 'systemctl start' for headless).
#                                          -d disowns the VM and exits, so the
#                                          wrapper can fire-and-forget.
#   nixling down  <vm>            -- stop cleanly
#   nixling status [<vm>]         -- show service / process / SSH health
#   nixling usb   <vm>            -- YubiKey USBIP attach (requires
#                                    nixling.vms.<vm>.usbip.yubikey)
#   nixling console <vm>          -- foreground serial console (headless
#                                    VMs only; graphics has its own window)
{ lib, pkgs, config, ... }:

let
  nl = import ./lib.nix { inherit lib pkgs; };

  enabledVms = lib.filterAttrs (_: vm: vm.enable) config.nixling.vms;

  # Per-VM manifest entries are assembled by nixos-modules/manifest.nix
  # (the documented Rust-CLI-ready JSON contract); cli.nix only
  # consumes them via `config.nixling.manifest.<name>` for the per-VM
  # Konsole launcher's SSH coordinates, and via the rendered JSON file
  # for the shell-level jq filters baked into the `nixling` wrapper.
  manifest = config.nixling.manifest;

  # W4-followup H2 (security): SSH private-key paths are NOT in the
  # public manifest — that file is world-readable and exposing a
  # secret-material path would leak it. The CLI instead resolves the
  # key path locally from `nixling.site.keysDir` (or the per-VM
  # `ssh.keyPath` override) at Nix-eval time and bakes a static
  # per-VM lookup into the shell wrapper below.
  resolveSshKeyPath = name: vm:
    if vm.ssh.keyPath != null
    then vm.ssh.keyPath
    else "${config.nixling.site.keysDir}/${name}_ed25519";

  vmSshKeyPaths = lib.mapAttrs resolveSshKeyPath enabledVms;

  # Bash case-statement body resolving `$VM -> <key-path>` for every
  # enabled VM. Folded into the `vm_ssh_key` shell helper.
  vmSshKeyCaseBody =
    lib.concatStringsSep "\n"
      (lib.mapAttrsToList
        (n: keyPath:
          "          ${lib.escapeShellArg n}) printf '%s\\n' ${lib.escapeShellArg keyPath} ;;")
        vmSshKeyPaths);

  storeSyncPkg = config.nixling.store.package;

  # §4.5 audit subcommand — constants baked at Nix eval time.
  auditChVersion      = "52.0";
  auditCrosvmRev      = "4c80bf3523cf84114054209d88a7af3eefd8423f";
  auditSeccompRev     = "299c1e7c3d5a1b98106212c20f58b9fdb7b1b1ea";
  # True at eval time iff nixpkgs' crosvm.src.rev still matches the
  # CH52-tested rev above.  Surfaced in 'nixling audit' JSON.
  auditChCrosvmPairOk =
    if auditCrosvmRev == pkgs.crosvm.src.rev then "true" else "false";

  nixling = pkgs.writeShellApplication {
    name = "nixling";
    # Suppress shellcheck warnings that flag false positives in this
    # script — most are jq-parsed vars in for-loops (SC2154) and
    # word-splitting that we want intentionally (SC2086 on $VM_PID).
    excludeShellChecks = [ "SC2154" ];
    runtimeInputs = with pkgs; [
      coreutils
      util-linux
      iproute2
      systemd
      gnugrep
      gawk
      procps
      openssh
      jq
      linuxPackages.usbip
      kmod
      nix
      curl
      acl
      storeSyncPkg
    ];
    text = ''
      set -euo pipefail

      MANIFEST=${config.nixling._manifestJsonPath}
      FLAKE_DEFAULT=${if config.nixling.site.flakePath == null then "" else config.nixling.site.flakePath}
      STATE_ROOT=${config.nixling.store.stateDir}

      usage() {
        cat <<EOF
nixling: manage nixling microVMs.

Subcommands:
  list                  List declared VMs and their capabilities.
  up   <vm> [-d|--detach]
                        Bring up (interactive for graphics VMs,
                        systemd-managed otherwise). With -d the
                        wrapper disowns the cloud-hypervisor child
                        and exits, leaving the VM running. Stop
                        later with 'nixling down <vm>'.
  down    <vm> [--force]  Stop cleanly. Use --force to stop a net VM
                          while its env's workload VMs are still up.
  restart <vm> [--force]  Stop cleanly, then start. Convenience wrapper
                          around \`down <vm>\` + \`up <vm>\`. For graphics
                          VMs you must invoke from a Wayland session
                          (same as plain \`up\`). \`--force\` is passed
                          through to the down step. Idempotent: a
                          stopped VM is just brought up.
                          Use this to apply pending unit-file changes
                          after \`nixos-rebuild switch\` (when
                          \`nixling list\` flags \`[pending restart]\`).
                          For VM-NixOS-module edits, see \`switch\` below
                          (and docs/reference/cli-contract.md
                          "restart vs switch").
  status  [<vm>]        Service / process / SSH health. Always
                        appends a per-bridge health section.
  status  --check-bridges
                        ONLY print the per-bridge health table; exit
                        non-zero (4) if any \`br-<env>-{up,lan}\`
                        bridge is admin-down, missing, or in
                        no-carrier while its env's net VM is
                        running. Report columns are
                        \`BRIDGE | STATE | ADMIN | EXPECTED | RESULT\`;
                        operstate=UP/DOWN/UNKNOWN is interpreted
                        against the live net-VM/workload state
                        (a stopped net VM yields
                        \`no-carrier (net VM stopped)\`, rc=0).
  usb     <vm>          YubiKey USBIP attach (Ctrl-C to detach).
                        Only one env may hold a device at a time;
                        switching requires: nixling usb <other-vm>.
  console <vm>          Foreground serial console (headless VMs).
  audio   <subcmd> ...  Per-VM mic + speaker grant/revoke. See
                        \`nixling audio --help\`. Requires the VM to
                        have \`nixling.vms.<vm>.audio.enable = true\`.

Lifecycle subcommands (per-VM closures + live activation):
  build       <vm>          Build the VM's closure on the host. Prints the
                            generation derivation path; symlinks
                            \$STATE_ROOT/<vm>/result for host GC root.
  switch      <vm>          build + sync per-VM /nix/store + run
                            \`switch-to-configuration switch\` over SSH.
                            Live activation: no VM reboot, default boot
                            updated. Default fast-path for VM edits.
                            Use \`switch\` when you edited the VM's own
                            NixOS module; use \`restart\` (above) when
                            only the framework's unit files moved during
                            a host \`nixos-rebuild switch\`.
  boot        <vm>          build + sync + bump default boot only (no live
                            activation). Takes effect on next start.
  test        <vm>          build + sync + live \`switch-to-configuration
                            test\`. Does NOT bump default boot.
  rollback    <vm>          Roll the in-VM system profile back one
                            generation and re-activate live.
  generations <vm>          List per-VM nix-profile generations
                            (in-VM) + per-VM store-meta generations
                            (host-side).
  gc          <vm>          Re-run the retention sweep on the per-VM
                            store; reclaims old hardlinks and
                            generation metadata. Safe on a running VM.

Host-key pinning (M2):
  trust            <vm>     Scan and pin the VM's SSH host key into
                            \$STATE_ROOT/known_hosts.nixling (TOFU on
                            first contact).  Run once per VM after
                            first boot; re-run after an intentional
                            rebuild that rotates host keys.
  rotate-known-host <vm>   Remove the stale host-key entry for <vm>
                            and prompt to re-run 'trust' after the
                            VM reboots.

Framework-managed SSH keys:
  keys list [--json]       Show every declared VM's framework-managed
                           pubkey fingerprint, on-disk path, age.
  keys show <vm>           Print the public key for <vm>.
  keys rotate <vm>         Generate a fresh keypair for <vm>; push +
                           verify the new pubkey via SSH using the
                           old key; stash the old key under
                           <keysDir>/old/<timestamp>/.

Security:
  audit [--strict] [--human]
                        Emit a JSON security-posture report (§4.5 of
                        SECURITY-nixling.md). --strict exits non-zero
                        if any field deviates from the post-hardening
                        target state. --human (default on tty) outputs
                        a human-readable summary instead of raw JSON.
EOF
        exit "''${1:-0}"
      }

      vm_exists() {
        # The reserved `_manifest` sentinel key carries the schema
        # version; treat it as a non-VM so `nixling up _manifest`
        # produces the same "unknown VM" error as any other typo.
        case "$1" in _*) return 1 ;; esac
        jq -e --arg n "$1" '.[$n] != null' "$MANIFEST" >/dev/null
      }

      vm_get() {
        # vm_get <vm> <jq-path>     -> string value or 'null'
        jq -r --arg n "$1" ".[\$n].$2" "$MANIFEST"
      }

      # W4-followup H2 (security): SSH private-key paths are baked
      # into the wrapper at Nix-eval time instead of being read from
      # the world-readable manifest. Emits the resolved private-key
      # path for <vm> on stdout, or `null` if <vm> is not a known
      # nixling-managed VM. The case body is generated by cli.nix
      # from `nixling.site.keysDir` + per-VM `ssh.keyPath` overrides.
      vm_ssh_key() {
        local VM="$1"
        case "$VM" in
${vmSshKeyCaseBody}
          *) printf '%s\n' "null" ;;
        esac
      }

      # Enumerate user-declared VM names from the manifest, skipping
      # the reserved `_manifest` schema-version sentinel (and any
      # future reserved `_*` keys). All `keys[]` iterations in this
      # CLI route through this helper so adding new top-level meta
      # fields doesn't accidentally feed them in as VM names.
      manifest_vms() {
        jq -r 'keys[] | select(startswith("_") | not)' "$MANIFEST"
      }

      # C4: reject empty or non-[a-zA-Z0-9_-] identifiers before
      # passing values into privileged sudo heredoc arguments.
      assert_safe() {
        case "$1" in
          ''''''|*[!a-zA-Z0-9_-]*)
            echo "nixling: unsafe identifier: $1" >&2
            exit 2
            ;;
        esac
      }

      require_vm() {
        if [ -z "''${1:-}" ]; then
          echo "nixling: missing <vm> argument" >&2; exit 2
        fi
        if ! vm_exists "$1"; then
          echo "nixling: unknown VM '$1'" >&2
          # Plain comma-space inside the jq filter — Nix indented
          # strings don't escape backslashes the way shell heredocs
          # do, so the previous `\",\\\\ \"` literal was passed to jq
          # verbatim and tripped its parser whenever require_vm
          # tried to print the candidate list.
          echo "  declared: $(manifest_vms | paste -sd, - | sed 's/,/, /g')" >&2
          exit 2
        fi
      }

      # All PIDs associated with $1 (systemd-launched microvm@,
      # interactive cloud-hypervisor with this VM's nix closure on
      # its cmdline, and the crosvm GPU sidecar). One source of
      # truth so down-guard and status agree about "is it running".
      vm_pids() {
        pgrep -f "microvm@$1\\b|nixos-system-$1-|crosvm device .*$1-gpu\\.sock" 2>/dev/null || true
      }

      vm_running() {
        [ -n "$(vm_pids "$1")" ]
      }

      # Liveness probe across the wrapper + backend. Returns 0 when
      # either `nixling@<vm>.service` (the user-facing wrapper) OR
      # `microvm@<vm>.service` (the backend / implementation-detail
      # template from microvm.nix) is active. Used by lifecycle
      # commands (`switch`, `boot`, `test`, `rollback`,
      # `generations`) to decide "is this VM currently up?". The
      # wrapper's BindsTo cascade normally keeps them in lockstep,
      # but mid-upgrade / mid-rollback may show only one side
      # active; checking both is the safer default.
      vm_active() {
        systemctl is-active --quiet "nixling@$1.service" 2>/dev/null \
          || systemctl is-active --quiet "microvm@$1.service" 2>/dev/null
      }

      # Ensure the net VM for $1's env is up before continuing.
      # Headless net VMs run as systemd units (autostart=true), so
      # a simple `systemctl start` is idempotent. We target the
      # user-facing wrapper `nixling@<net-vm>.service`; its
      # ExecStart propagates to the underlying `microvm@<net-vm>`
      # (implementation detail — see host.nix wrapper template).
      ensure_net_vm_up() {
        local rvm
        rvm=$(vm_get "$1" netVm)
        if [ "$rvm" = "null" ] || [ -z "$rvm" ]; then
          return 0
        fi
        if systemctl is-active --quiet "nixling@$rvm.service"; then
          return 0
        fi
        echo "nixling: starting net VM '$rvm' for env"
        sudo -A systemctl start "nixling@$rvm.service"
        # Wait for the net VM's LAN-side default route to be answerable.
        local rip
        rip=$(jq -r --arg e "$(vm_get "$1" env)" \
          '.[] | select(.isNetVm == true and .env == $e) | .staticIp' \
          "$MANIFEST" | head -1)
        if [ -n "$rip" ] && [ "$rip" != "null" ]; then
          echo -n "nixling: waiting for $rvm sshd at $rip"
          for _ in $(seq 1 60); do
            if timeout 1 bash -c "</dev/tcp/$rip/22" 2>/dev/null; then
              echo " — ready."; return 0
            fi
            sleep 1; echo -n "."
          done
          echo
          echo "nixling: warning — $rvm:22 didn't answer within 60s; continuing anyway"
        fi
      }

      do_list() {
        printf '%-18s %-9s %-9s %-5s %-7s %-15s %s\n' \
          NAME ENV GRAPHICS TPM USBIP STATIC_IP STATUS
        local _any_pending=0
        for vm in $(manifest_vms); do
          env=$(vm_get "$vm" env)
          [ "$env" = "null" ] && env="-"
          g=$(vm_get "$vm" graphics)
          t=$(vm_get "$vm" tpm)
          u=$(vm_get "$vm" usbipYubikey)
          ip=$(vm_get "$vm" staticIp)
          [ "$ip" = "null" ] && ip="(dhcp)"
          if [ "$(vm_get "$vm" isNetVm)" = "true" ]; then
            tag="net-vm"
          else
            tag=""
          fi
          # Prefer the user-facing wrapper for the live-state check;
          # fall back to the microvm@ backend so VMs started by older
          # `nixling up` flows (or directly via `systemctl start
          # microvm@<vm>`) still show as running.
          if systemctl is-active --quiet "nixling@$vm.service" 2>/dev/null \
             || systemctl is-active --quiet "microvm@$vm.service" 2>/dev/null; then
            st=systemd
          elif vm_running "$vm"; then
            st=interactive
          else
            st=stopped
          fi
          # v0.1.5: surface pending config changes. With
          # X-RestartIfChanged=false on per-VM sidecars, a
          # nixos-rebuild updates the unit files but does NOT
          # cycle the running VM. Compare the per-VM `current`
          # symlink (latest declared closure) against `booted`
          # (the closure the running VM actually executed). If
          # they differ AND the VM is running, the consumer
          # needs `nixling restart <vm>` to apply changes (a
          # clean down+up cycles the running closure over the
          # already-staged new unit files). `nixling switch <vm>`
          # is for the different case of editing the VM's own
          # NixOS module — see docs/reference/cli-contract.md
          # ("restart vs switch").
          if vm_pending_restart "$vm"; then
            st="$st [pending restart]"
            _any_pending=1
          fi
          [ -n "$tag" ] && st="$st ($tag)"
          printf '%-18s %-9s %-9s %-5s %-7s %-15s %s\n' \
            "$vm" "$env" "$g" "$t" "$u" "$ip" "$st"
        done
        if [ "$_any_pending" -eq 1 ]; then
          echo
          echo "(one or more VMs have unapplied unit-file changes; run \`nixling restart <vm>\` to apply)"
        fi
      }

      # v0.1.5: detect whether <vm> is running an out-of-date closure.
      # Returns 0 (true) when the per-VM `current` symlink points at a
      # different store path than `booted` AND the VM is up.
      # Returns 1 (false) otherwise:
      #  - VM stopped (nothing booted; not "pending").
      #  - `booted` missing (graphics VMs pre-v0.1.5; first-run case).
      #  - `current` == `booted` (no change needed).
      vm_pending_restart() {
        local _vm="$1"
        local _statedir="/var/lib/nixling/vms/$_vm"
        local _booted_target _current_target
        _booted_target=$(readlink "$_statedir/booted" 2>/dev/null) || return 1
        _current_target=$(readlink "$_statedir/current" 2>/dev/null) || return 1
        [ -z "$_booted_target" ] || [ -z "$_current_target" ] && return 1
        [ "$_booted_target" = "$_current_target" ] && return 1
        # Different. Is the VM actually running?
        if systemctl is-active --quiet "nixling@$_vm.service" 2>/dev/null \
           || systemctl is-active --quiet "microvm@$_vm.service" 2>/dev/null \
           || vm_running "$_vm"; then
          return 0
        fi
        return 1
      }


      # ---------------------------------------------------------------------------
      # P2r4 security-r4-1: host-wide USBIP exclusive-attach/cleanup helpers.
      # fd 9 must be opened on /run/nixling/usbipd.lock before calling attach.
      # The lock file is pre-created root:nixling-launcher 0660 by
      # systemd.tmpfiles (host.nix) so nixling-launcher members can open
      # it with exec 9> without write access to the root:root 0755 /run/nixling.
      # ---------------------------------------------------------------------------
      usbip_exclusive_attach() {
        # usbip_exclusive_attach <target_env> <busid> <all_envs>
        local _ue_env="$1" _ue_busid="$2" _ue_all_envs="$3"
        # P2r6 security-r6-1: USBIP_LOCK_HELD must NOT be honoured from the
        # caller environment; bash imports inherited env vars as shell vars,
        # so `USBIP_LOCK_HELD=1 nixling usb ...` would make cleanup believe
        # the lock was held even after flock fails. Reset to 0 BEFORE flock.
        USBIP_LOCK_HELD=0
        flock -w 30 9 || {
          echo "nixling: could not acquire /run/nixling/usbipd.lock; another USB session active" >&2
          exit 5
        }
        USBIP_LOCK_HELD=1
        # C4: validate env identifiers before privileged use.
        assert_safe "$_ue_env"
        for _safe_env in $_ue_all_envs; do assert_safe "$_safe_env"; done
        echo "nixling: enforcing exclusive USB export for env '$_ue_env' (all envs: $_ue_all_envs)..."
        sudo -A bash -s -- "$_ue_env" "$_ue_all_envs" <<${"'"}BASH${"'"}
          set -euo pipefail
          UE_ENV=$1; UE_ALL_ENVS=$2
          for env in $UE_ALL_ENVS; do
            [ "$env" = "$UE_ENV" ] && continue
            systemctl stop "nixling-sys-$env-usbipd-proxy.socket"          2>/dev/null || true
            systemctl stop "nixling-sys-$env-usbipd-proxy.service"         2>/dev/null || true
            systemctl stop "nixling-sys-$env-usbipd-backend.service" 2>/dev/null || true
          done
          systemctl start "nixling-sys-$UE_ENV-usbipd-proxy.socket"
BASH
        echo "nixling: binding $_ue_busid to usbip-host (detaches from host xhci)..."
        sudo -A bash -s -- "$_ue_env" "$_ue_busid" <<${"'"}BASH${"'"}
          set -euxo pipefail
          UE_ENV=$1; UE_BUSID=$2
          modprobe usbip-host
          systemctl start "nixling-sys-$UE_ENV-usbipd-backend.service"
          /run/current-system/sw/bin/usbip unbind -b "$UE_BUSID" 2>/dev/null || true
          /run/current-system/sw/bin/usbip bind -b "$UE_BUSID"
BASH
      }

      usbip_exclusive_cleanup() {
        # usbip_exclusive_cleanup <target_env> <busid> <all_envs>
        local _uc_env="$1" _uc_busid="$2" _uc_all_envs="$3"
        # P2r5 nixos-r5-1/security-r5-1: skip if flock was never acquired (failed
        # contender must not tear down the active holder's bind).
        if [ "''${USBIP_LOCK_HELD:-0}" != "1" ]; then
          echo "nixling: usbip_exclusive_cleanup: skipping, lock was not acquired" >&2
          return 0
        fi
        # C4: validate env identifier before privileged use.
        assert_safe "$_uc_env"
        for _safe_env in $_uc_all_envs; do assert_safe "$_safe_env"; done
        sudo -A bash -s -- "$_uc_env" "$_uc_busid" "$_uc_all_envs" <<${"'"}BASH${"'"}
          /run/current-system/sw/bin/usbip unbind -b "$2" 2>/dev/null || true
          systemctl stop "nixling-sys-$1-usbipd-backend.service" 2>/dev/null || true
          systemctl stop "nixling-sys-$1-usbipd-proxy.service"         2>/dev/null || true
          # restore non-target env sockets
          UC_ENV=$1; UC_ALL_ENVS=$3
          for env in $UC_ALL_ENVS; do
            [ "$env" = "$UC_ENV" ] && continue
            systemctl start "nixling-sys-$env-usbipd-proxy.socket" 2>/dev/null || true
          done
BASH
      }

      # ----------------------------------------------------------------------
      # Interactive bring-up for graphics VMs.
      # The whole flow needs WAYLAND_DISPLAY + XDG_RUNTIME_DIR (for the
      # crosvm GPU sidecar to find the host compositor) and the kvm
      # group (for /var/lib/microvms state and /dev/kvm). Cleanup on
      # any exit path: VM kill, GPU sidecar kill, virtiofsd stop,
      # swtpm stop, tap delete, stale-socket sweep, optional USBIP
      # release.
      # ----------------------------------------------------------------------
      do_up_graphics() {
        # Most of these survive into the EXIT-trap cleanup (which
        # fires from the outer scope after do_up_graphics returns),
        # so we deliberately don't `local` them.
        local BRIDGE
        VM="$1"
        local DETACH="''${2:-false}"
        TAP=$(vm_get "$VM" tap)
        BRIDGE=$(vm_get "$VM" bridge)
        STATE=$(vm_get "$VM" stateDir)
        SOCK=$(vm_get "$VM" apiSocket)
        TPM=$(vm_get "$VM" tpm)
        TP_SOCK=$(vm_get "$VM" tpmSocket)
        USBIPYK=$(vm_get "$VM" usbipYubikey)
        STATIC_IP=$(vm_get "$VM" staticIp)
        SSH_USER=$(vm_get "$VM" sshUser)
        SSH_KEY=$(vm_ssh_key "$VM")
        USBIP_BUSID=""
        VM_PID=""
        # C4: validate identifier values before privileged use.
        # NOTE: STATE is an absolute path (e.g. /var/lib/nixling/vms/<vm>) and is
        # NOT an identifier; it's safe via quoted heredoc positional-arg use.
        # Only validate identifiers used in shell-sensitive tokens or unit names.
        assert_safe "$VM"
        assert_safe "$TAP"
        assert_safe "$BRIDGE"

        if [ -z "''${WAYLAND_DISPLAY:-}" ] || [ -z "''${XDG_RUNTIME_DIR:-}" ]; then
          echo "nixling: '$VM' is a graphics VM and must be launched from a Wayland session" >&2
          echo "  (WAYLAND_DISPLAY and XDG_RUNTIME_DIR must be set)" >&2
          exit 1
        fi
        if [ "$(id -un)" = "root" ]; then
          echo "nixling: do NOT run as root; run as your Plasma user." >&2
          exit 1
        fi
        # P4 C3/H5: the host user is not in the kvm group; check nixling-launcher instead.
        # The nixling-launcher group grants polkit access to start/stop nixling
        # service units. kvm access goes to the dedicated nixling-<vm>-gpu user.
        # If the user IS in the group declaratively but the current shell
        # session was started before the group was added (kernel only refreshes
        # supplementary groups at session-start), auto-re-exec via `sg` so the
        # user doesn't have to log out + back in to apply a fresh nixos-rebuild.
        if ! id -Gn | tr ' ' '\n' | grep -qx nixling-launcher; then
          if id -Gn "$(id -un)" 2>/dev/null | tr ' ' '\n' | grep -qx nixling-launcher; then
            echo "nixling: nixling-launcher group not active in current shell — re-execing via sg nixling-launcher..." >&2
            local _d_flag=""
            [ "$DETACH" = "true" ] && _d_flag=" -d"
            exec /run/wrappers/bin/sg nixling-launcher -c "$0 up $VM$_d_flag"
          fi
          echo "nixling: $(id -un) is not in the nixling-launcher group." >&2
          echo "  Add $(id -un) to nixling-launcher in configuration.nix and re-run nixos-rebuild." >&2
          exit 5
        fi

        # If a previous headless run left the wrapper / microvm@ up,
        # tear it down before relaunching as a graphics VM. The
        # wrapper's ExecStop propagates the stop to microvm@<vm>;
        # checking microvm@ directly catches mismatched states where
        # the wrapper went away but the backend is still running.
        if systemctl is-active --quiet "nixling@$VM.service" \
           || systemctl is-active --quiet "microvm@$VM.service"; then
          echo "nixling: stopping nixling@$VM.service (and backend microvm@$VM.service) first..."
          sudo -A systemctl stop "nixling@$VM.service" 2>/dev/null || true
          sudo -A systemctl stop "microvm@$VM.service" 2>/dev/null || true
        fi

        # security-r8-audio-9: idempotent up — if the VM's GPU sidecar
        # is already active AND its CH API socket exists, the VM is
        # already running and we should NOT tear it down. Without this
        # guard, clicking the .desktop launcher (which calls
        # `nixling up <vm> -d`) on an already-running VM would kill
        # the running CH, delete its tap, restart virtiofsd, and try
        # to bring it back up — racing with the still-resident GPU
        # sidecar. Observed as a "flashing chromeless
        # terminal": the visible artifact of the VM being repeatedly
        # killed and partially re-spawned by the launcher.
        if systemctl is-active --quiet "nixling-$VM-gpu.service" \
           && [ -S "$SOCK" ]; then
          echo "nixling: '$VM' is already running (gpu sidecar active, API socket present); no-op."
          # Idempotent ensure: if audio state changed, the sidecar
          # restart on a fresh `up` is needed to attach --generic-
          # vhost-user; we deliberately SKIP that here. If the operator
          # toggles audio while the VM is up, they must
          # `nixling down <vm> && nixling up <vm>` (documented
          # elsewhere — CH v52 doesn't support live add/remove of
          # generic-vhost-user devices).
          if [ "$DETACH" = "true" ]; then
            exit 0
          fi
          # Foreground mode: wait for the VM to exit (matches the
          # non-idempotent path which `wait`s on the runner).
          while sudo -A systemctl is-active --quiet "nixling-$VM-gpu.service" 2>/dev/null; do
            sleep 1
          done
          exit 0
        fi

        # The workload VM expects its env's net VM (DHCP + default
        # gateway + DNS) to be up before it boots. ensure_net_vm_up
        # is a no-op for VMs without an env (legacy) or for net VMs
        # themselves.
        ensure_net_vm_up "$VM"

        # Pre-launch sweep. Reap any prior nixling-up wrappers AND any
        # microvm runners + crosvm GPU sidecars left over from a crashed
        # or killed run. The wrapper reap is critical: a still-running
        # OLD wrapper's EXIT trap (which fires when its `wait $VM_PID`
        # returns because we killed its VM) would race-`systemctl stop`
        # the virtiofsd / swtpm we're about to use.
        reaped_pids=""
        for pid in $(pgrep -af "nixling up $VM\b" 2>/dev/null | grep -v "^$$ " | awk '{print $1}'); do
          [ "$pid" = "$$" ] && continue
          echo "nixling: reaping concurrent wrapper pid=$pid"
          kill "$pid" 2>/dev/null || true
          reaped_pids="$reaped_pids $pid"
        done
        # Wait for the reaped wrappers to fully exit. SIGTERM triggers
        # their EXIT trap, which `systemctl stop`s virtiofsd/swtpm and
        # deletes the tap. If we proceed to our own `systemctl restart
        # virtiofsd` before that finishes, the old wrapper's belated
        # stop wins and kills the unit we just brought up — the
        # symptom is "Job for microvm-virtiofsd@<vm>.service canceled"
        # and a half-built VM that immediately falls over.
        for pid in $reaped_pids; do
          for _ in $(seq 1 40); do
            [ -d "/proc/$pid" ] || break
            sleep 0.25
          done
        done
        for pid in $(vm_pids "$VM"); do
          echo "nixling: reaping orphan pid=$pid from prior run"
          kill "$pid" 2>/dev/null || true
        done
        for _ in 1 2 3; do
          sleep 1
          vm_running "$VM" || break
        done
        for pid in $(vm_pids "$VM"); do
          kill -9 "$pid" 2>/dev/null || true
        done

        echo "nixling: prepping host (tap''${TPM:+, swtpm}, virtiofsd)..."
        # P4 C3: CH now runs as nixling-$VM-gpu; the TAP must be owned by that
        # user so CH (running as nixling-$VM-gpu) can open /dev/net/tun.
        _GPU_USER="nixling-$VM-gpu"
        sudo -A bash -s -- "$TAP" "$BRIDGE" "$VM" "$STATE" "$TPM" "$TP_SOCK" "$_GPU_USER" <<${"'"}BASH${"'"}
          set -euo pipefail
          TAP=$1; BRIDGE=$2; VM=$3; STATE=$4; TPM=$5; TP_SOCK=$6; NL_USER=$7
          if [ -e "/sys/class/net/$TAP" ]; then
            ip link delete "$TAP"
          fi
          ip tuntap add name "$TAP" mode tap user "$NL_USER" vnet_hdr multi_queue
          ip link set "$TAP" master "$BRIDGE"
          ip link set "$TAP" up
          rm -f "$STATE/$VM-gpu.sock" "$STATE/$VM.sock"
          find "$STATE" -maxdepth 1 -name "$VM-virtiofs-*.sock" -delete 2>/dev/null || true
          systemctl restart "microvm-virtiofsd@$VM.service"
          for _ in $(seq 1 20); do
            [ -S "$STATE/$VM-virtiofs-ro-store.sock" ] && break
            sleep 0.25
          done
          if [ "$TPM" = "true" ]; then
            systemctl restart "nixling-$VM-swtpm.service"
            for _ in $(seq 1 20); do
              [ -S "$TP_SOCK" ] && break
              sleep 0.25
            done
            if ! [ -S "$TP_SOCK" ]; then
              echo "nixling: swtpm socket $TP_SOCK did not appear" >&2
              exit 1
            fi
          fi
BASH

        cleanup() {
          local rc=$?
          # Re-enable :- defaults so an EXIT trap firing before vars are
          # set doesn't trip `set -u`.
          local VM="''${VM:-}"
          local USBIP_BUSID="''${USBIP_BUSID:-}"
          local VM_PID="''${VM_PID:-}"
          local STATIC_IP="''${STATIC_IP:-null}"
          local SSH_USER="''${SSH_USER:-null}"
          local SSH_KEY="''${SSH_KEY:-null}"
          local TPM="''${TPM:-false}"
          local TAP="''${TAP:-}"
          local STATE="''${STATE:-/var/lib/microvms/$VM}"
          # P2r3 nixos-2/security-2: capture usbip env context for cleanup.
          local UP_ENV="''${ENV:-}"
          local UP_ALL_ENVS="''${UP_ALL_ENVS:-}"
          if [ -n "$USBIP_BUSID" ] && [ "$STATIC_IP" != "null" ] && \
             [ "$SSH_USER" != "null" ] && [ "$SSH_KEY" != "null" ]; then
            ssh -i "$SSH_KEY" \
                -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
                -o ConnectTimeout=3 \
                "$SSH_USER@$STATIC_IP" "
                  port=\$(sudo /run/current-system/sw/bin/usbip port 2>/dev/null \
                    | grep -oE 'Port [0-9]+' | head -1 | awk '{print \$2}')
                  [ -n \"\$port\" ] && sudo /run/current-system/sw/bin/usbip detach -p \$port || true
                " 2>/dev/null || true
          fi
          # P4 C3: stop the nixling-$VM-gpu.service (which owns crosvm-gpu + CH).
          # Also kill by PID for resilience (service might already be in failed state).
          sudo -A systemctl stop "nixling-''${VM:-}-gpu.service" 2>/dev/null || true
          kill "''${VM_PID:-}" 2>/dev/null || true
          for pid in $(vm_pids "$VM"); do
            kill "$pid" 2>/dev/null || true
          done
          for _ in 1 2 3; do
            sleep 1
            vm_running "$VM" || break
          done
          for pid in $(vm_pids "$VM"); do
            kill -9 "$pid" 2>/dev/null || true
          done
          sudo -A bash -s -- "$VM" "$TAP" "$STATE" "$TPM" <<${"'"}BASH${"'"}
            VM=$1; TAP=$2; STATE=$3; TPM=$4
            systemctl stop "nixling-$VM-gpu.service" 2>/dev/null || true
            systemctl stop "microvm-virtiofsd@$VM.service" 2>/dev/null || true
            if [ "$TPM" = "true" ]; then
              systemctl stop "nixling-$VM-swtpm.service" 2>/dev/null || true
            fi
            if [ -e "/sys/class/net/$TAP" ]; then
              ip link delete "$TAP" || true
            fi
            rm -f "$STATE/$VM-gpu.sock" "$STATE/$VM.sock"
            find "$STATE" -maxdepth 1 -name "$VM-virtiofs-*.sock" -delete 2>/dev/null || true
BASH

          # P2r4 security-r4-1: shared cleanup helper releases USB lock + restores sockets.
          if [ -n "$USBIP_BUSID" ] && [ -n "$UP_ENV" ]; then
            usbip_exclusive_cleanup "$UP_ENV" "$USBIP_BUSID" "$UP_ALL_ENVS"
          fi
          # P4 C3: audio sidecar is now a system service (not in the user manager).
          # In detach mode, leave it running so the VM keeps audio.
          if [ "$DETACH" != "true" ] && [ "$(vm_get "$VM" audio)" = "true" ]; then
            sudo -A systemctl stop "nixling-''${VM:-}-snd.service" 2>/dev/null || true
          fi
          exit $rc
        }
        trap cleanup EXIT INT TERM

        # P4 C3: launch via nixling-$VM-gpu.service (runs as nixling-$VM-gpu,
        # not the host user). The service handles Wayland/kvm ACLs in ExecStartPre.
        #
        # security-r8-audio-2: pre-start the audio sidecar HERE (under
        # the nixling-launcher polkit grant) when the VM has audio
        # enabled and at least one direction is on. Otherwise the
        # audioArgsScript inside microvm-run (running as nixling-$VM-gpu,
        # a system user with no polkit grant) tries to start the unit
        # itself — which pops a polkit password dialog on the operator's
        # session every time the VM comes up. Also: the sidecar's
        # ExecStartPost now blocks until /run/nixling/vms/$VM/snd.sock
        # exists and has the nixling-$VM-gpu ACL applied (audio-host.nix),
        # so by the time this `systemctl start` returns CH can
        # deterministically connect.
        #
        # security-r8-audio-4: ALWAYS RESTART (not just start) the
        # sidecar. vhost-device-sound v0.3.0 is single-connection by
        # design: once CH connects, the listen socket is consumed and
        # the per-virtqueue vring_worker threads bind to that
        # connection. When CH exits (VM shutdown), those threads panic
        # on the closed peer and the sidecar process is left
        # half-dead — the main thread + PipeWire thread survive, but
        # vring_workers are gone, so a NEW CH connection negotiates
        # virtio messages that nobody answers, and arecord/aplay in
        # the guest time out with `Connection timed out`. Restarting
        # the unit gives every new VM lifetime a clean worker pool.
        if [ "$(vm_get "$VM" audio)" = "true" ]; then
          local _a_mic _a_spk
          read -r _a_mic _a_spk < <(audio_read "$VM" 2>/dev/null || echo "off off")
          if [ "$_a_mic" = "on" ] || [ "$_a_spk" = "on" ]; then
            echo "nixling: restarting nixling-$VM-snd.service for audio (mic=$_a_mic speaker=$_a_spk)..."
            audio_sidecar_restart "$VM" || {
              echo "nixling: failed to restart nixling-$VM-snd.service; CH will be launched without audio" >&2
            }
          fi
        fi

        echo "nixling: launching nixling-$VM-gpu.service ..."
        sudo -A systemctl reset-failed "nixling-$VM-gpu.service" 2>/dev/null || true
        sudo -A systemctl start "nixling-$VM-gpu.service"
        VM_PID=""  # no background PID; service managed by systemd

        echo -n "nixling: waiting for $SOCK"
        for _ in $(seq 1 120); do
          if [ -S "$SOCK" ]; then echo " — ready."; break; fi
          if ! sudo -A systemctl is-active --quiet "nixling-$VM-gpu.service" 2>/dev/null; then
            echo
            echo "nixling: nixling-$VM-gpu.service stopped before API socket appeared" >&2
            sudo -A systemctl status "nixling-$VM-gpu.service" --no-pager -n 20 >&2 || true
            exit 4
          fi
          sleep 0.5; echo -n "."
        done
        if [ ! -S "$SOCK" ]; then
          echo; echo "nixling: timed out waiting for $SOCK" >&2; exit 4
        fi

        # Optional YubiKey USBIP attach at boot. The user can also do
        # this later via the 'nixling usb VM' subcommand.
        if [ "$USBIPYK" = "true" ]; then
          for d in /sys/bus/usb/devices/*; do
            [ -f "$d/idVendor" ] || continue
            if [ "$(cat "$d/idVendor")" = "1050" ]; then
              USBIP_BUSID=$(basename "$d"); break
            fi
          done
          if [ -n "$USBIP_BUSID" ] && [ "$STATIC_IP" != "null" ] && \
             [ "$SSH_USER" != "null" ] && [ "$SSH_KEY" != "null" ]; then
            local ENV
            ENV=$(vm_get "$VM" env)
            echo "nixling: USBIPing YubiKey ($USBIP_BUSID) into VM..."
            # P2r4 security-r4-1: acquire host-wide lock + exclusive export via shared helper.
            local UP_ALL_ENVS
            UP_ALL_ENVS=$(jq -r '[.[].env] | map(select(. != null)) | unique | .[]' "$MANIFEST" | tr '\n' ' ')
            exec 9>/run/nixling/usbipd.lock
            usbip_exclusive_attach "$ENV" "$USBIP_BUSID" "$UP_ALL_ENVS"
            echo -n "nixling: waiting for VM sshd"
            for _ in $(seq 1 60); do
              if timeout 1 bash -c "</dev/tcp/$STATIC_IP/22" 2>/dev/null; then
                echo " — ready."; break
              fi
              sleep 1; echo -n "."
            done
            ssh -i "$SSH_KEY" \
                -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
                -o ConnectTimeout=10 \
                "$SSH_USER@$STATIC_IP" "
                  sudo /run/current-system/sw/bin/modprobe vhci_hcd
                  sudo /run/current-system/sw/bin/usbip attach -r $(vm_get "$VM" usbipdHostIp) -b $USBIP_BUSID
                " || echo "nixling: YubiKey attach failed (non-fatal)"
            echo "nixling: YubiKey attached."
          fi
        fi

        cat <<EOF

  $VM is up.
    SSH:     $([ "$SSH_USER" != "null" ] && echo "ssh -i $SSH_KEY $SSH_USER@$STATIC_IP" || echo "(no ssh credentials declared)")
    Console: appears as a Wayland window in your Plasma session
    YubiKey: $([ -n "$USBIP_BUSID" ] && echo "attached via USBIP ($USBIP_BUSID)" || echo "not plugged in (re-run nixling up after inserting, or use nixling usb $VM)")
    Shutdown: poweroff from inside the VM, or close the window
$( [ "$DETACH" = "true" ] && echo "    Mode:    detached — VM keeps running after this wrapper exits; stop with 'nixling down $VM'." )
$( [ "$DETACH" = "true" ] && echo "    Logs:    $STATE/$VM.log" )

EOF
        if [ "$DETACH" = "true" ]; then
          # P4 C3: nixling-$VM-gpu.service runs independently of this wrapper.
          # Disarm the EXIT trap so cleanup() doesn't stop the service when we
          # exit. The YubiKey USBIP busid stays bound so the YubiKey remains
          # attached to the VM after we exit.
          trap - EXIT INT TERM
          exit 0
        fi
        # Non-detach: block until the service stops (VM shutdown or error).
        while sudo -A systemctl is-active --quiet "nixling-$VM-gpu.service" 2>/dev/null; do
          sleep 2
        done
      }

      do_up_headless() {
        VM="$1"
        ensure_net_vm_up "$VM"
        echo "nixling: starting nixling@$VM.service via systemd..."
        sudo -A systemctl start "nixling@$VM.service"
        sudo -A systemctl status "nixling@$VM.service" --no-pager -n 5
      }

      do_up() {
        require_vm "''${1:-}"
        VM="$1"
        local DETACH="''${2:-false}"
        if [ "$(vm_get "$VM" graphics)" = "true" ]; then
          do_up_graphics "$VM" "$DETACH"
        else
          # Headless VMs are already systemd-managed (microvm@<vm>.
          # service), so they're inherently detached from the caller.
          # -d is a no-op there; we accept it silently for symmetry.
          do_up_headless "$VM"
        fi
      }

      # v0.1.5: convenience wrapper. With X-RestartIfChanged=false on
      # the per-VM sidecars, a nixos-rebuild leaves running VMs on
      # the OLD closure; the user has to manually cycle one. The
      # nixling status / list `pending-restart` indicator tells the
      # user WHICH VMs need this; `nixling restart <vm>` performs
      # the cycle in one step. Graphics VMs still require a Wayland
      # session (the up step would error otherwise).
      #
      # Optional second arg `--force` is forwarded to `down` so net
      # VMs can be restarted without first stopping the env's
      # workloads.
      do_restart() {
        require_vm "''${1:-}"
        VM="$1"
        local FORCE="''${2:-}"
        echo "nixling: restarting '$VM' (down then up)..."
        # Down is a no-op if already stopped, so no need to test
        # state first. The function logs its own progress.
        do_down "$VM" "$FORCE"
        # Brief settle so any teardown side effects (tap removal,
        # virtiofsd socket cleanup) finish before we re-up.
        ${pkgs.coreutils}/bin/sleep 1
        # Detach mode is not propagated — restart is interactive by
        # default for graphics VMs (same as a fresh `up`). Headless
        # VMs are systemd-managed regardless.
        do_up "$VM" false
      }

      do_down() {
        require_vm "''${1:-}"
        VM="$1"
        local FORCE="''${2:-}"
        local TAP STATE TPM IS_NET_VM ENV
        TAP=$(vm_get "$VM" tap)
        STATE=$(vm_get "$VM" stateDir)
        TPM=$(vm_get "$VM" tpm)
        IS_NET_VM=$(vm_get "$VM" isNetVm)
        ENV=$(vm_get "$VM" env)
        # C4: validate identifier values before privileged use.
        # NOTE: STATE is an absolute path, not an identifier — already safe via
        # quoted heredoc positional args.
        assert_safe "$VM"
        assert_safe "$TAP"
        [ "$ENV" = "null" ] || assert_safe "$ENV"

        # If this is a net VM, refuse to stop while any workload
        # VM in the same env is still running (their network would
        # break). Override with --force.
        if [ "$IS_NET_VM" = "true" ] && [ "$FORCE" != "--force" ]; then
          local active_workloads=""
          for peer in $(jq -r --arg e "$ENV" \
              '. | to_entries | map(select(.value.env == $e and .value.isNetVm == false)) | .[].key' \
              "$MANIFEST"); do
            if systemctl is-active --quiet "nixling@$peer.service" 2>/dev/null \
               || systemctl is-active --quiet "microvm@$peer.service" 2>/dev/null \
               || vm_running "$peer"; then
              active_workloads="$active_workloads $peer"
            fi
          done
          if [ -n "$active_workloads" ]; then
            echo "nixling: refusing to stop net VM '$VM'; workload VMs still up:$active_workloads" >&2
            echo "  Stop the workload VMs first, or pass --force to override." >&2
            exit 4
          fi
        fi

        echo "nixling: stopping $VM..."
        # P4 B: stop GPU sidecar first (CH runs inside it; microvm@ manages state).
        if systemctl is-active --quiet "nixling-$VM-gpu.service" 2>/dev/null; then
          sudo -A systemctl stop "nixling-$VM-gpu.service" 2>/dev/null || true
        fi
        # Stop via the nixling@<vm> wrapper. Its ExecStop /
        # PropagatesStopTo cascades to microvm@<vm> (backend /
        # implementation detail). Falling through to microvm@ on
        # the bare-microvm case (wrapper not loaded) keeps the
        # CLI tolerant of mismatched generations.
        if systemctl is-active --quiet "nixling@$VM.service"; then
          sudo -A systemctl stop "nixling@$VM.service"
        elif systemctl is-active --quiet "microvm@$VM.service"; then
          sudo -A systemctl stop "microvm@$VM.service"
        fi
        for pid in $(vm_pids "$VM"); do
          kill "$pid" 2>/dev/null || true
        done
        sleep 1
        for pid in $(vm_pids "$VM"); do
          kill -9 "$pid" 2>/dev/null || true
        done
        sudo -A bash -s -- "$VM" "$TAP" "$STATE" "$TPM" <<${"'"}BASH${"'"}
          VM=$1; TAP=$2; STATE=$3; TPM=$4
          systemctl stop "microvm-virtiofsd@$VM.service" 2>/dev/null || true
          if [ "$TPM" = "true" ]; then
            systemctl stop "nixling-$VM-swtpm.service" 2>/dev/null || true
          fi
          if [ -e "/sys/class/net/$TAP" ]; then
            ip link delete "$TAP" || true
          fi
          rm -f "$STATE/$VM-gpu.sock" "$STATE/$VM.sock"
          find "$STATE" -maxdepth 1 -name "$VM-virtiofs-*.sock" -delete 2>/dev/null || true
BASH


        # P4 C3: audio sidecar is now a system service (nixling-$VM-snd.service).
        # Stop it regardless of calling user.
        if [ "$(vm_get "$VM" audio)" = "true" ]; then
          sudo -A systemctl stop "nixling-$VM-snd.service" 2>/dev/null || true
        fi
        echo "nixling: $VM stopped."
      }

      # ----------------------------------------------------------------------
      # Bridge health (M5; networking-1 in P1r1 hardening).
      #
      # The per-env bridges (br-<env>-up + br-<env>-lan) are declared
      # in network.nix and are the only path workload VMs have to the
      # outside world. networkd-wait-online is disabled in this config
      # (see network.nix), so a bridge that has been administratively
      # brought down (e.g. `ip link set br-... down` by mistake, or a
      # networkd misconfig) goes unnoticed -- workload VMs lose
      # connectivity silently.
      #
      # The kernel exposes two state fields per link:
      #
      #   * Admin flag (IFF_UP) -- the literal token `UP` inside the
      #     `<...>` flag list in `ip -o link show`. Cleared means the
      #     bridge is administratively down (`ip link set ... down`).
      #
      #   * Operstate -- `state X` after the flag list. For a bridge
      #     this is `UP` when at least one enslaved member has
      #     carrier, `DOWN` when no member has carrier (net VM
      #     stopped, all taps detached), occasionally `UNKNOWN` for
      #     interfaces that don't report carrier (treated like DOWN
      #     here: "no carrier currently flowing").
      #
      # The original M5 implementation keyed health on admin only,
      # which mis-classified a real outage as healthy: a bridge with
      # admin=UP but operstate=DOWN (net VM crashed, tap got
      # detached) reported `ok`. networking-1 fixes this by
      # cross-referencing operstate with the expected state derived
      # from the manifest + live systemd state:
      #
      #   * `br-<env>-up` (host<->net-VM p2p): if the env's net VM
      #     is active, operstate MUST be UP (the net VM's u2 tap
      #     carries the link). If the net VM is stopped, operstate
      #     DOWN/UNKNOWN is expected and reported as
      #     `no-carrier (net VM stopped)` with rc=0.
      #
      #   * `br-<env>-lan` (workload LAN): if the env's net VM is
      #     active AND at least one workload VM in that env is
      #     active, operstate MUST be UP. Otherwise no-carrier is
      #     expected (annotated `net VM stopped` or `no workloads up`).
      #
      # Returns 0 if every bridge meets its expected state. Returns
      # 4 if any bridge is admin-down, missing, or operstate!=UP
      # while UP was expected.
      do_check_bridges() {
        local rc=0 br info state admin env kind net_vm net_vm_active
        local any_workload_active expected result reason w
        local bridges
        bridges=$(jq -r '[.[] | .bridge] | map(select(. != null)) | unique | .[]' "$MANIFEST")
        echo "=== Bridge health ==="
        printf '%-20s %-10s %-7s %-12s %s\n' \
          "BRIDGE" "STATE" "ADMIN" "EXPECTED" "RESULT"
        if [ -z "$bridges" ]; then
          echo "(no bridges declared in manifest)"
          return 0
        fi
        while IFS= read -r br; do
          [ -z "$br" ] && continue
          if ! info=$(ip -o link show "$br" 2>/dev/null); then
            printf '%-20s %-10s %-7s %-12s %s\n' \
              "$br" "MISSING" "missing" "?" "FAIL (missing)"
            rc=4
            continue
          fi
          state="?"
          if [[ "$info" =~ state[[:space:]]+([A-Z]+) ]]; then
            state="''${BASH_REMATCH[1]}"
          fi
          # Match IFF_UP as a discrete token inside the <...> flag
          # list: literal `UP` bracketed by `<`/`,` on the left and
          # `,`/`>` on the right.
          admin="DOWN"
          if [[ "$info" =~ [\<,]UP[,\>] ]]; then
            admin="up"
          fi

          # Parse env + kind from bridge name (br-<env>-up | br-<env>-lan).
          # Bridges that don't match this naming convention are
          # reported as informational (no expected state derivable).
          env=""
          kind=""
          if [[ "$br" =~ ^br-(.+)-(up|lan)$ ]]; then
            env="''${BASH_REMATCH[1]}"
            kind="''${BASH_REMATCH[2]}"
          fi

          net_vm=""
          net_vm_active=false
          any_workload_active=false
          if [ -n "$env" ]; then
            net_vm=$(jq -r --arg e "$env" \
              '[.[] | select(.isNetVm == true and .env == $e) | .name] | .[0] // ""' \
              "$MANIFEST")
            # Check both the user-facing wrapper and the microvm@
            # backend so we register the net VM as "active" even if
            # the wrapper is missing (e.g. mid-upgrade).
            if [ -n "$net_vm" ] \
               && (systemctl is-active --quiet "nixling@$net_vm.service" 2>/dev/null \
                   || systemctl is-active --quiet "microvm@$net_vm.service" 2>/dev/null); then
              net_vm_active=true
            fi
            while IFS= read -r w; do
              [ -z "$w" ] && continue
              if systemctl is-active --quiet "nixling@$w.service" 2>/dev/null \
                 || systemctl is-active --quiet "microvm@$w.service" 2>/dev/null \
                 || vm_running "$w"; then
                any_workload_active=true
                break
              fi
            done < <(jq -r --arg e "$env" \
              '.[] | select(.isNetVm == false and .env == $e) | .name' \
              "$MANIFEST")
          fi

          # Determine the operstate we EXPECT given live net-VM /
          # workload state. `UP` means "must have carrier" (a real
          # outage if it isn't UP). `NO-CARRIER` means "no carrier
          # is the normal resting state" (UP is still fine; DOWN/
          # UNKNOWN is informational, not a failure).
          if [ -z "$env" ] || [ -z "$kind" ]; then
            expected="?"
          elif [ "$kind" = "up" ]; then
            if [ "$net_vm_active" = "true" ]; then expected="UP"
            else                                   expected="NO-CARRIER"
            fi
          else
            # lan
            if [ "$net_vm_active" = "true" ] && [ "$any_workload_active" = "true" ]; then
              expected="UP"
            else
              expected="NO-CARRIER"
            fi
          fi

          # Score result. admin-down is always a failure (someone
          # `ip link set ... down`'d the bridge by hand). When
          # expected=UP, operstate MUST be UP. When expected=NO-CARRIER,
          # both UP (extra activity, fine) and DOWN/UNKNOWN
          # (resting) are healthy; we just annotate the reason.
          if [ "$admin" != "up" ]; then
            result="FAIL (admin down)"
            rc=4
          elif [ "$expected" = "UP" ]; then
            if [ "$state" = "UP" ]; then
              result="ok"
            else
              result="FAIL (no-carrier, net VM up)"
              rc=4
            fi
          elif [ "$expected" = "NO-CARRIER" ]; then
            if [ "$state" = "UP" ]; then
              result="ok"
            else
              if [ "$net_vm_active" != "true" ]; then
                reason="net VM stopped"
              else
                reason="no workloads up"
              fi
              result="no-carrier ($reason)"
            fi
          else
            result="(informational)"
          fi

          printf '%-20s %-10s %-7s %-12s %s\n' \
            "$br" "$state" "$admin" "$expected" "$result"
        done <<< "$bridges"
        return "$rc"
      }

      do_status() {
        local check_only=false vm_arg=""
        while [ $# -gt 0 ]; do
          case "$1" in
            --check-bridges) check_only=true; shift ;;
            --)              shift
                             if [ $# -gt 0 ]; then
                               if [ -z "$vm_arg" ]; then vm_arg="$1"; shift
                               else echo "nixling status: unexpected extra argument '$1'" >&2; exit 2
                               fi
                             fi
                             break ;;
            -*)              echo "nixling status: unknown flag '$1'" >&2; exit 2 ;;
            *)               if [ -z "$vm_arg" ]; then vm_arg="$1"; shift
                             else echo "nixling status: unexpected extra argument '$1'" >&2; exit 2
                             fi ;;
          esac
        done

        if [ "$check_only" = "true" ]; then
          if [ -n "$vm_arg" ]; then
            echo "nixling status: --check-bridges takes no <vm> argument" >&2
            exit 2
          fi
          do_check_bridges
          return $?
        fi

        if [ -z "$vm_arg" ]; then
          do_list
          echo
          do_check_bridges || true
          return 0
        fi

        require_vm "$vm_arg"
        VM="$vm_arg"
        local STATIC_IP SSH_USER
        STATIC_IP=$(vm_get "$VM" staticIp)
        SSH_USER=$(vm_get "$VM" sshUser)
        echo "=== $VM ==="
        # User-facing wrapper first; microvm@ is labelled "backend"
        # to mark it as the implementation detail (microvm.nix's
        # template that the wrapper drives via BindsTo + ExecStop).
        systemctl is-active "nixling@$VM.service" --no-pager 2>/dev/null \
          | sed "s|^|nixling@$VM: |"
        systemctl is-active "microvm@$VM.service" --no-pager 2>/dev/null \
          | sed "s|^|microvm@$VM (backend): |"
        systemctl is-active "microvm-virtiofsd@$VM.service" --no-pager 2>/dev/null \
          | sed "s|^|virtiofsd: |"
        if [ "$(vm_get "$VM" tpm)" = "true" ]; then
          systemctl is-active "nixling-$VM-swtpm.service" --no-pager 2>/dev/null \
            | sed "s|^|swtpm: |"
        fi
        if vm_running "$VM"; then
          echo "interactive: running"
        else
          echo "interactive: stopped"
        fi
        if [ "$STATIC_IP" != "null" ]; then
          if timeout 1 bash -c "</dev/tcp/$STATIC_IP/22" 2>/dev/null; then
            echo "sshd@$STATIC_IP:22: reachable''${SSH_USER:+ (user=$SSH_USER)}"
          else
            echo "sshd@$STATIC_IP:22: unreachable"
          fi
        fi
        # v0.1.5: pending-restart hint. Per-VM sidecars carry
        # X-RestartIfChanged=false, so a nixos-rebuild updates unit
        # files but does NOT bounce the running VM. Surface that
        # mismatch here so the user knows when `nixling restart <vm>`
        # is required to cycle the running closure (use `nixling
        # switch <vm>` only when editing the VM's own NixOS module —
        # see docs/reference/cli-contract.md "restart vs switch").
        if vm_pending_restart "$VM"; then
          local _booted _current
          _booted=$(readlink "/var/lib/nixling/vms/$VM/booted" 2>/dev/null || echo "(none)")
          _current=$(readlink "/var/lib/nixling/vms/$VM/current" 2>/dev/null || echo "(none)")
          echo "pending-restart: YES — unit files changed; run \`nixling restart $VM\` to apply"
          echo "  booted : $_booted"
          echo "  current: $_current"
        else
          echo "pending-restart: no"
        fi
        echo
        do_check_bridges || true
        return 0
      }

      do_usb() {
        require_vm "''${1:-}"
        VM="$1"
        if [ "$(vm_get "$VM" usbipYubikey)" != "true" ]; then
          echo "nixling: '$VM' does not have usbip.yubikey enabled" >&2
          exit 2
        fi
        local STATIC_IP SSH_USER SSH_KEY HOST_IP ENV
        STATIC_IP=$(vm_get "$VM" staticIp)
        SSH_USER=$(vm_get "$VM" sshUser)
        SSH_KEY=$(vm_ssh_key "$VM")
        HOST_IP=$(vm_get "$VM" usbipdHostIp)
        ENV=$(vm_get "$VM" env)
        # C4: validate identifier values before privileged use (usbip helper passes ENV in unit names).
        assert_safe "$VM"
        [ "$ENV" = "null" ] || assert_safe "$ENV"
        if [ "$STATIC_IP" = "null" ] || [ "$SSH_USER" = "null" ] || [ "$SSH_KEY" = "null" ]; then
          echo "nixling: '$VM' needs staticIp + ssh.user + ssh.keyPath set to use 'usb'" >&2
          exit 2
        fi
        if [ "$HOST_IP" = "null" ] || [ -z "$HOST_IP" ]; then
          echo "nixling: '$VM' has no usbipdHostIp (env not set?). Set nixling.vms.$VM.env." >&2
          exit 2
        fi
        if [ "$ENV" = "null" ] || [ -z "$ENV" ]; then
          echo "nixling: '$VM' has no env — cannot pick a per-env usbipd service." >&2
          exit 2
        fi

        DEV_SYS=""
        for d in /sys/bus/usb/devices/*; do
          [ -f "$d/idVendor" ] || continue
          if [ "$(cat "$d/idVendor")" = "1050" ]; then
            DEV_SYS="$d"; break
          fi
        done
        if [ -z "$DEV_SYS" ]; then
          echo "nixling: no Yubico USB device plugged into the host." >&2
          exit 2
        fi
        BUSID=$(basename "$DEV_SYS")
        PRODUCT=$(cat "$DEV_SYS/product" 2>/dev/null || echo Yubico)
        echo "nixling: found $PRODUCT at busid $BUSID"

        if ! ssh -i "$SSH_KEY" \
                -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
                -o ConnectTimeout=5 \
                "$SSH_USER@$STATIC_IP" true 2>/dev/null; then
          echo "nixling: VM at $STATIC_IP is not reachable. Run 'nixling up $VM' first." >&2
          exit 3
        fi

        # P2r4 nixos-2/rust-r4-1 + security-r4-1: usbipd.lock pre-created root:kvm 0660
        # by tmpfiles; exec 9> succeeds for kvm-group members.  Shared helper acquires
        # flock + enforces exclusive export (fixes both missing-file and missing-flock).
        ALL_ENVS=$(jq -r '[.[].env] | map(select(. != null)) | unique | .[]' "$MANIFEST" | tr '\n' ' ')
        exec 9>/run/nixling/usbipd.lock

        cleanup_usb() {
          local rc=$?
          echo
          echo "nixling: releasing YubiKey..."
          ssh -i "$SSH_KEY" \
              -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
              -o ConnectTimeout=3 \
              "$SSH_USER@$STATIC_IP" "
                set +e
                port=\$(sudo /run/current-system/sw/bin/usbip port 2>/dev/null \
                  | grep -oE 'Port [0-9]+' | head -1 | awk '{print \$2}')
                if [ -n \"\$port\" ]; then
                  sudo /run/current-system/sw/bin/usbip detach -p \$port || true
                fi
              " 2>/dev/null || true
          usbip_exclusive_cleanup "$ENV" "$BUSID" "$ALL_ENVS"
          echo "nixling: YubiKey returned to host."
          exit $rc
        }
        trap cleanup_usb EXIT INT TERM

        # P2r4 security-r4-1: shared helper acquires flock + enforces exclusive export + binds.
        usbip_exclusive_attach "$ENV" "$BUSID" "$ALL_ENVS"
        echo "nixling: hot-plugging $BUSID into VM..."
        ssh -i "$SSH_KEY" \
            -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
            "$SSH_USER@$STATIC_IP" "sudo /run/current-system/sw/bin/modprobe vhci_hcd && \
                                    sudo /run/current-system/sw/bin/usbip attach -r $HOST_IP -b $BUSID"

        cat <<EOF

  YubiKey is now in the $VM VM via USBIP.
    Host: YubiKey detached from xhci_hcd (mouse + everything else unaffected)
    VM:   lsusb should show $PRODUCT; /dev/hidrawN appears

  Ctrl-C to detach (returns YubiKey to host).
EOF
        for _usb_i in $(seq 1 100); do sleep 60; done
      }

      do_console() {
        require_vm "''${1:-}"
        VM="$1"
        if [ "$(vm_get "$VM" graphics)" = "true" ]; then
          echo "nixling: '$VM' is a graphics VM; use 'nixling up $VM' instead" >&2
          exit 2
        fi
        local STATE
        STATE=$(vm_get "$VM" stateDir)
        # C4: validate VM identifier before privileged use.
        assert_safe "$VM"
        sudo -A bash -s -- "$STATE" "$VM" <<${"'"}BASH${"'"}
          STATE=$1; VM=$2
          cd "$STATE" && exec microvm -r "$VM"
BASH
      }

      # ----------------------------------------------------------------------
      # Audio: explicit per-direction grant/revoke of host mic + speaker.
      #
      # State of record: /var/lib/nixling/vms/<vm>/state/audio-state.json, one of
      #   {"mic":"off","speaker":"off"}    # no audio device
      #   {"mic":"on" ,"speaker":"off"}    # mic only
      #   {"mic":"off","speaker":"on" }    # speaker only
      #   {"mic":"on" ,"speaker":"on" }    # both
      #
      # The state file is the single source of truth. The boot-time
      # extraArgsScript (modules/nixling/audio.nix) reads it to decide
      # whether to attach a virtio-snd device, and the WirePlumber rule
      # in modules/nixling/audio-host.nix uses it as a fallback policy.
      #
      # All file I/O is atomic (write-temp-then-rename) and serialised
      # per-VM via flock on /var/lib/nixling/vms/<vm>/audio.lock. The CLI
      # also tries `ch-remote add-device / remove-device` for live
      # hot-add/hot-remove when the device-attached state crosses; if
      # that's unsupported by the running cloud-hypervisor (v50 with
      # the spectrum patches), it falls back to telling the user to
      # restart the VM.
      # ----------------------------------------------------------------------

      # shellcheck source=/dev/null
      . ${nl.nixlingReadAudioState}

      # P4 C3: audio sidecar is now a system service (nixling-<vm>-snd).
      # Socket is at /run/nixling/vms/<vm>/snd.sock (RuntimeDirectory).
      audio_socket() {
        echo "/run/nixling/vms/$1/snd.sock"
      }

# Read the on-disk state for $1. Echoes "<mic> <speaker>" with each
      # field "on" or "off". Delegates to nixling_read_audio_state (sourced
      # from lib.nix above) for fail-closed, normalised reads.
      audio_read() {
        local _ar_result _ar_mic _ar_spk
        _ar_result=$(nixling_read_audio_state "$1")
        _ar_mic=''${_ar_result#mic=}; _ar_mic=''${_ar_mic% *}
        _ar_spk=''${_ar_result#* speaker=}
        printf '%s %s\n' "$_ar_mic" "$_ar_spk"
      }

      # Atomically write state. Args: <vm> <mic> <speaker>.
      audio_write() {
        local vm="$1" mic="$2" spk="$3" f lock json
        f=$(vm_get "$vm" audioStateFile)
        local d parent
        d=$(dirname "$f")           # /var/lib/nixling/vms/<vm>/state
        parent=$(dirname "$d")      # /var/lib/nixling/vms/<vm>
        # P3r2 software-1: reject a pre-existing symlink at $d. The parent
        # dir is kvm-writable (microvm:kvm 2775) so any kvm-group process
        # could pre-create state/ as a symlink to an attacker-chosen path.
        # `[ -d "$d" ]` follows symlinks, so the original guard would treat
        # an attacker symlink as "already a directory" and skip the install
        # step; the privileged install would then write audio-state.json
        # outside the intended root:kvm 0750 dir. Fail closed.
        if [ -L "$d" ]; then
          echo "nixling: audio state path $d is a symlink (refusing to write through it)" >&2
          return 1
        fi
        if [ -e "$d" ] && [ ! -d "$d" ]; then
          echo "nixling: audio state path $d exists but is not a directory" >&2
          return 1
        fi
        if [ ! -d "$d" ]; then
          # P4: state/ subdir is root:nixling-launcher 0750; parent is microvm:microvm 0750.
          # Ensure the parent dir exists first (microvm-managed), then create state/.
          if [ ! -d "$parent" ]; then
            sudo -A install -d -m 2770 -o microvm -g kvm "$parent"
          fi
          sudo -A install -d -m 0750 -o root -g nixling-launcher "$d"  # P4: group→nixling-launcher
        fi
        # P3r2 software-1: defense-in-depth — verify final ownership/mode AFTER
        # install. If kvm-group raced an unsafe state dir in between, abort.
        local stat_d
        stat_d=$(stat -c '%U %G %a' "$d" 2>/dev/null || echo "missing")
        if [ "$stat_d" != "root nixling-launcher 750" ]; then
          echo "nixling: audio state dir $d has unsafe perms ($stat_d), refusing" >&2
          return 1
        fi
        # P4: lock in /run/nixling/ (root:nixling-launcher 0660) so nixling-launcher members
        # (nixling-launcher member) can open it without kvm-group membership.
        lock="/run/nixling/audio-$vm.lock"
        json=$(printf '{"mic":"%s","speaker":"%s"}\n' "$mic" "$spk")
        # P2r4 software-1: write tempfile inside the root-owned state/ dir (0750 root:kvm)
        # so no kvm-group process can swap it for a symlink between create and install.
        # P3r3 software-r3-1: parent /var/lib/nixling/vms/<vm> is kvm-writable, so a
        # kvm-group process can rename/replace state/ between the outer stat-check
        # and the privileged write. Re-validate state/ identity INSIDE the privileged
        # critical section: assert it's a regular directory (not a symlink), owned
        # root:kvm with mode 0750, exactly matching the expected path.
        (
          flock 9
          sudo -A bash -s -- "$d" "$f" "$json" "$vm" <<${"'"}BASH${"'"}
            set -euo pipefail
            d=$1; f=$2; json=$3; vm=$4
            # P3r5 rust-r5-1 / software-r5-1: between [-L "$d"] check and `cd`,
            # an attacker can swap state/ with a symlink. `cd` follows symlinks
            # even if our path-based -L check passed. Defense: capture the inode
            # of $d BEFORE cd, then assert post-cd cwd has the SAME inode. If
            # state/ was renamed/replaced with a symlink in between, our cd will
            # resolve to a different inode, and the comparison fails.
            if [ -L "$d" ]; then
              echo "nixling: $d is a symlink before cd; refusing privileged write" >&2
              exit 1
            fi
            pre_inode=$(stat -c '%i' -- "$d" 2>/dev/null || echo "")
            pre_dev=$(stat -c '%d' -- "$d" 2>/dev/null || echo "")
            if [ -z "$pre_inode" ] || [ -z "$pre_dev" ]; then
              echo "nixling: cannot stat $d; refusing" >&2
              exit 1
            fi
            cd -- "$d" || {
              echo "nixling: cannot chdir to $d; refusing privileged write" >&2
              exit 1
            }
            post_inode=$(stat -c '%i' -- . 2>/dev/null || echo "")
            post_dev=$(stat -c '%d' -- . 2>/dev/null || echo "")
            if [ "$pre_inode" != "$post_inode" ] || [ "$pre_dev" != "$post_dev" ]; then
              echo "nixling: $d inode changed during cd (pre=$pre_dev:$pre_inode post=$post_dev:$post_inode); refusing" >&2
              exit 1
            fi
            stat_d=$(stat -c '%U %G %a' -- . 2>/dev/null || echo "missing")
            if [ "$stat_d" != "root nixling-launcher 750" ]; then
              echo "nixling: cwd perms changed under us ($stat_d), refusing" >&2
              exit 1
            fi
            # Verify the basename of $f matches our expected canonical name.
            base=$(basename -- "$f")
            if [ "$base" != "audio-state.json" ]; then
              echo "nixling: unexpected basename $base, refusing" >&2
              exit 1
            fi
            # Write tempfile in the anchored cwd, then atomic-rename via the
            # anchored cwd. install/mv with no '/' in source resolves the
            # tempfile via the cwd, not via $d as a re-resolved path.
            tmp=$(mktemp ./audio-state.json.XXXXXX)
            printf '%s\n' "$json" > "$tmp"
            install -m 0640 -o root -g nixling-launcher "$tmp" "./$base"
            # P7r1 software-1: reapply nixling-<vm>-gpu:r ACL — install
            # replaces the inode so the activation-time ACL is gone.
            # nixling-<vm>-gpu needs to read audio-state.json at VM start.
            setfacl -m "u:nixling-''${vm}-gpu:r" "./$base" || true
            rm -f "$tmp"
BASH
        ) 9>"$lock"
      }

      # Try to hot-attach the audio device to a running VM. Returns 0 if
      # the device is now attached, 2 if hotplug isn't supported (caller
      # falls back to "restart required" message), 1 on other error.
      audio_hotplug_add() {
        local vm="$1" sock api_sock
        sock=$(audio_socket "$vm") || return 1
        api_sock=$(vm_get "$vm" apiSocket)
        if [ ! -S "$api_sock" ]; then
          return 1   # VM isn't running
        fi
        # cloud-hypervisor v50's add-device endpoint accepts a JSON
        # payload of {"path":"..."} for vfio/pci devices. For generic
        # vhost-user the supported shape is not documented; we attempt
        # it but expect it may fail. Either way, fall through to
        # "restart required" on non-success.
        if curl -sS --fail --unix-socket "$api_sock" \
             -H 'Content-Type: application/json' \
             -X PUT "http://localhost/api/v1/vm.add-device" \
             -d "{\"socket\":\"$sock\",\"id\":\"nixling-$vm-snd\"}" \
             >/dev/null 2>&1; then
          return 0
        fi
        return 2
      }

      audio_hotplug_remove() {
        local vm="$1" api_sock
        api_sock=$(vm_get "$vm" apiSocket)
        if [ ! -S "$api_sock" ]; then
          return 1
        fi
        if curl -sS --fail --unix-socket "$api_sock" \
             -H 'Content-Type: application/json' \
             -X PUT "http://localhost/api/v1/vm.remove-device" \
             -d "{\"id\":\"nixling-$vm-snd\"}" \
             >/dev/null 2>&1; then
          return 0
        fi
        return 2
      }

      # P4 C3: audio sidecar is now a system service; use systemctl (no --user).
      audio_sidecar_start() {
        sudo -A systemctl reset-failed "nixling-$1-snd.service" 2>/dev/null || true
        sudo -A systemctl start "nixling-$1-snd.service"
      }

      # security-r8-audio-4: ALWAYS restart on every VM boot —
      # vhost-device-sound's vring_workers panic when the previous CH
      # peer disconnects, leaving the unit "active" but functionally
      # dead for the next CH instance. Restart wipes those workers and
      # creates a fresh listen socket for the new CH to claim.
      audio_sidecar_restart() {
        sudo -A systemctl reset-failed "nixling-$1-snd.service" 2>/dev/null || true
        sudo -A systemctl restart "nixling-$1-snd.service"
      }

      audio_sidecar_stop() {
        sudo -A systemctl stop "nixling-$1-snd.service" 2>/dev/null || true
      }

      audio_sidecar_active() {
        systemctl is-active --quiet "nixling-$1-snd.service"
      }

      # WirePlumber reload was a hook for a future stream-rule; v1
      # ships no rule, so reloading is purely cost. We MUST NOT just
      # `kill -HUP` WirePlumber: the upstream code path treats SIGHUP
      # like SIGTERM and exits ("stopped by signal: Hangup"), which
      # leaves the host without a session manager — ALSA cards
      # disappear from plasma-pa and only `systemctl --user restart
      # wireplumber` recovers them. This bug was the cause of two
      # confirmed host-audio outages in May 2026. Don't reintroduce
      # SIGHUP-based reload here; if/when a stream-rule lands, drive
      # it via a wpctl-based settings update or a Lua script load,
      # never SIGHUP.
      audio_wireplumber_reload() {
        :   # intentional no-op (see comment above)
      }

      # Status for one VM. Prints lines:
      #   audio:    enabled | not-enabled
      #   mic:      on | off
      #   speaker:  on | off
      #   sidecar:  active | inactive | not-applicable
      #   device:   attached | detached | unknown
      audio_status_one() {
        local vm="$1" cap mic spk sidecar device
        cap=$(vm_get "$vm" audio)
        if [ "$cap" != "true" ]; then
          printf 'audio:    not-enabled\n'
          return 0
        fi
        read -r mic spk < <(audio_read "$vm")
        if audio_sidecar_active "$vm" 2>/dev/null; then
          sidecar=active
        else
          sidecar=inactive
        fi
        if [ "$mic" = "off" ] && [ "$spk" = "off" ]; then
          device=detached
        elif vm_running "$vm"; then
          device=attached
        else
          device="will-attach-on-next-up"
        fi
        printf 'audio:    enabled\n'
        printf 'mic:      %s\n' "$mic"
        printf 'speaker:  %s\n' "$spk"
        printf 'sidecar:  %s\n' "$sidecar"
        printf 'device:   %s\n' "$device"
      }

      do_audio() {
        # nixling audio                       -> status for every VM
        # nixling audio status [<vm>]
        # nixling audio mic     on|off <vm>
        # nixling audio speaker on|off <vm>
        # nixling audio off     <vm>          (== mic off + speaker off)
        local sub="''${1:-status}"; shift || true

        case "$sub" in
          status)
            if [ -z "''${1:-}" ]; then
              for v in $(jq -r 'to_entries[] | select(.value.audio == true) | .key' "$MANIFEST"); do
                echo "=== $v ==="
                audio_status_one "$v"
                echo
              done
              return 0
            fi
            require_vm "$1"
            audio_status_one "$1"
            return 0
            ;;

          mic|speaker)
            local dir="$sub" state="''${1:-}" vm="''${2:-}"
            if [ -z "$state" ] || [ -z "$vm" ]; then
              echo "nixling audio: usage: nixling audio $dir on|off <vm>" >&2
              exit 2
            fi
            case "$state" in on|off) ;; *)
              echo "nixling audio: state must be 'on' or 'off'" >&2; exit 2 ;;
            esac
            require_vm "$vm"
            if [ "$(vm_get "$vm" audio)" != "true" ]; then
              echo "nixling audio: '$vm' does not have audio.enable=true." >&2
              echo "  Set nixling.vms.$vm.audio.enable = true; in modules/nixling/vms.nix, commit, rebuild, retry." >&2
              exit 2
            fi
            local old_mic old_spk new_mic new_spk
            read -r old_mic old_spk < <(audio_read "$vm")
            new_mic="$old_mic"; new_spk="$old_spk"
            if [ "$dir" = "mic" ]; then new_mic="$state"; else new_spk="$state"; fi
            _do_audio_apply "$vm" "$old_mic" "$old_spk" "$new_mic" "$new_spk"
            ;;

          off)
            local vm="''${1:-}"
            if [ -z "$vm" ]; then
              echo "nixling audio: usage: nixling audio off <vm>" >&2; exit 2
            fi
            require_vm "$vm"
            if [ "$(vm_get "$vm" audio)" != "true" ]; then
              # Idempotent: "off" on a never-enabled VM is a no-op.
              echo "nixling audio: '$vm' has audio.enable=false; nothing to do."
              return 0
            fi
            local old_mic old_spk
            read -r old_mic old_spk < <(audio_read "$vm")
            _do_audio_apply "$vm" "$old_mic" "$old_spk" "off" "off"
            ;;

          ""|-h|--help|help)
            cat <<EOF
nixling audio: per-VM mic + speaker grant/revoke.

Subcommands:
  status [<vm>]              Show current grant state. With no <vm>,
                             lists all VMs that have audio enabled.
  mic     on|off <vm>        Grant or revoke microphone access.
  speaker on|off <vm>        Grant or revoke speaker access.
  off            <vm>        Revoke both (same as mic off + speaker off).

Per-VM state lives at /var/lib/nixling/vms/<vm>/state/audio-state.json. Toggling
a VM that's currently running tries cloud-hypervisor hotplug first; if
that's not supported it tells you to restart the VM.
EOF
            return 0
            ;;

          *)
            echo "nixling audio: unknown subcommand '$sub'" >&2
            echo "  try: nixling audio --help" >&2
            exit 2
            ;;
        esac
      }

      # Apply a state transition. Args: <vm> <old_mic> <old_spk> <new_mic> <new_spk>.
      # Persists the new state, manages the sidecar service, attempts
      # CH hotplug if the device-attached state crosses, and reloads
      # WirePlumber. Prints the resulting status.
      _do_audio_apply() {
        local vm="$1" oM="$2" oS="$3" nM="$4" nS="$5"
        local was_on=false now_on=false running=false
        [ "$oM" = "on" ] || [ "$oS" = "on" ] && was_on=true
        [ "$nM" = "on" ] || [ "$nS" = "on" ] && now_on=true
        if vm_running "$vm"; then running=true; fi

        if [ "$oM" = "$nM" ] && [ "$oS" = "$nS" ]; then
          echo "nixling audio: no change (mic=$nM, speaker=$nS)."
          return 0
        fi

        audio_write "$vm" "$nM" "$nS"
        echo "nixling audio: state -> mic=$nM, speaker=$nS"

        # Sidecar lifecycle. Sidecar should run iff at least one
        # direction is on — BUT only when the VM is NOT running. If
        # the VM is up, the sidecar is the AF_UNIX peer of CH's vhost-
        # user connection; stopping it leaves CH with a dead peer and
        # any guest audio call (e.g. Firefox initialising WebAudio)
        # blocks forever. CH does not reconnect, so even restarting
        # the sidecar won't help. For a running VM the right scope
        # for "revoke speaker" is the state file (which we just
        # wrote) plus the WirePlumber stream rule (see audio-host.nix
        # for the input-direction null-target rule) — NOT process
        # teardown.
        #
        # Concrete failure mode this guard prevents:
        #   $ nixling audio off workload-vm   # while VM up
        #   ...sidecar stops...
        #   user opens Firefox -> WebAudio init -> writev() against
        #   the dead vhost-user socket -> uninterruptible D-state.
        if [ "$now_on" = "true" ]; then
          audio_sidecar_start "$vm"
        elif [ "$running" = "true" ]; then
          echo "nixling audio: VM is running; leaving sidecar alive (will stop on next nixling down)."
        else
          audio_sidecar_stop "$vm"
        fi

        # WirePlumber reload is a no-op in v1 (no rule installed).
        # Kept as a no-cost hook in case a stream-rule is added later.
        audio_wireplumber_reload || true

        # Hot-attach / hot-detach to a running VM when device-attached
        # state crosses.
        if [ "$running" = "true" ] && [ "$was_on" != "$now_on" ]; then
          local rc=0
          if [ "$now_on" = "true" ]; then
            audio_hotplug_add "$vm" || rc=$?
          else
            audio_hotplug_remove "$vm" || rc=$?
          fi
          case "$rc" in
            0)
              echo "nixling audio: hot-attached/detached on running VM."
              ;;
            *)
              echo "nixling audio: NOTE — this cloud-hypervisor build doesn't"
              echo "  support live add/remove of generic-vhost-user devices."
              echo "  The new state will take effect on next 'nixling down $vm && nixling up $vm'."
              ;;
          esac
        fi

        echo
        audio_status_one "$vm"
      }

      # ----------------------------------------------------------------------
      # Lifecycle subcommands: build / switch / boot / test / rollback /
      # generations / gc.
      # ----------------------------------------------------------------------

      # Resolve the flake to build from. NIXLING_FLAKE overrides the
      # default (which comes from `nixling.site.flakePath`, empty
      # when unset). Fail clearly if neither is provided.
      flake_target() {
        local f="''${NIXLING_FLAKE:-$FLAKE_DEFAULT}"
        if [ -z "$f" ]; then
          echo "nixling: no flake set — pass --flake or set NIXLING_FLAKE / nixling.site.flakePath" >&2
          exit 1
        fi
        echo "$f#nixling-$1"
      }

      # Build the per-VM closure generation derivation. Prints the
      # generation directory path on stdout. Side-effect: drops
      # \$STATE_ROOT/<vm>/result symlink as a GC root.
      do_build_inner() {
        local VM="$1"
        local STATE OUT
        STATE=$STATE_ROOT/$VM
        # `result` is the GC root. sudo because /var/lib/nixling/vms/<vm>
        # is microvm:kvm owned.
        sudo -A install -d -m 2775 -g kvm "$STATE"
        OUT=$STATE/result
        # `nix build` evaluates + builds; --no-link suppresses the local
        # `./result` symlink so we can place ours under STATE.
        local BUILD_TMP TARGET
        TARGET=$(flake_target "$VM")
        BUILD_TMP=$(sudo -A mktemp)
        if ! sudo -A bash -s -- "$TARGET" "$BUILD_TMP" <<${"'"}BASH${"'"}
          nix build --no-link --print-out-paths "$1" > "$2"
BASH
        then
          sudo -A rm -f "$BUILD_TMP"
          return 1
        fi
        local GEN
        GEN=$(sudo -A cat "$BUILD_TMP")
        sudo -A rm -f "$BUILD_TMP"
        sudo -A ln -sfT "$GEN" "$OUT"
        echo "$GEN"
      }

      do_build() {
        require_vm "''${1:-}"
        local VM="$1"
        echo "nixling: building $VM closure..."
        local GEN
        GEN=$(do_build_inner "$VM")
        echo "nixling: $VM closure → $GEN"
        echo "  GC root: $STATE_ROOT/$VM/result"
      }

      # Stage the generation pointer and run nixling-store-sync.
      sync_store() {
        local VM="$1" GEN="$2"
        sudo -A install -d -m 0755 "/run/nixling/$VM"
        sudo -A ln -sfT "$GEN" "/run/nixling/$VM/next-generation"
        sudo -A ${storeSyncPkg}/bin/nixling-store-sync "$VM" "$GEN"
      }

      # SSH into the VM and run a command. Errors with a helpful
      # message if ssh.user / ssh.keyPath aren't declared.
      require_ssh() {
        local VM="$1"
        local USER KEY IP
        USER=$(vm_get "$VM" sshUser)
        KEY=$(vm_ssh_key "$VM")
        IP=$(vm_get "$VM" staticIp)
        if [ "$USER" = "null" ] || [ "$KEY" = "null" ] || [ "$IP" = "null" ]; then
          echo "nixling: '$VM' has no ssh.user / ssh.keyPath / staticIp — cannot SSH for in-VM activation." >&2
          echo "  Declare them under nixling.vms.$VM.ssh.{user,keyPath} (and ensure env+index are set)." >&2
          exit 2
        fi
        echo "$USER@$IP $KEY"
      }

      vm_ssh() {
        local VM="$1"; shift
        local CRED USER_AT_IP KEY
        CRED=$(require_ssh "$VM")
        USER_AT_IP=$(echo "$CRED" | awk '{print $1}')
        KEY=$(echo "$CRED" | awk '{print $2}')
        ssh -i "$KEY" \
            -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
            -o ConnectTimeout=5 \
            "$USER_AT_IP" "$@"
      }

      # In-VM activate using a specific action.
      vm_activate() {
        local VM="$1" ACTION="$2"
        # /run/nixling-store-meta/current/system is the new toplevel
        # the host just synced. The guest has a
        # nixling-load-store-db.path unit that's supposed to fire on
        # every host-side db.dump update, but inotify across the
        # virtiofs host↔guest boundary is unreliable — `systemd.path`
        # frequently misses updates that the host wrote. So we force-
        # trigger the load service via SSH first to guarantee the new
        # paths are in the guest's nix DB before we try to activate.
        # The service is Type=oneshot so `systemctl start` blocks
        # until it has finished loading.
        #
        # Use `nix-store --query --is-valid` (the current flag) rather
        # than the pre-nix-2 `--query --valid`; the latter silently
        # fails with "unknown flag", which causes the wait-loop to
        # always think the path isn't ready and gives nix-env --set
        # the same path that the daemon then rejects.
        vm_ssh "$VM" "
          set -euo pipefail
          SYS=\$(readlink -f /run/nixling-store-meta/current/system)
          if [ -z \"\$SYS\" ] || [ ! -d \"\$SYS\" ]; then
            echo 'nixling: /run/nixling-store-meta/current/system missing in VM' >&2
            exit 1
          fi
          # Force-reload the per-VM closure into the guest nix DB.
          # Oneshot, so this blocks until done.
          sudo systemctl start nixling-load-store-db.service
          # Belt-and-suspenders: wait briefly in case the start raced
          # with an in-flight invocation from the path watcher.
          for _ in 1 2 3 4 5; do
            sudo /run/current-system/sw/bin/nix-store --query --is-valid \"\$SYS\" >/dev/null 2>&1 && break
            sleep 1
          done
          if [ '$ACTION' = 'switch' ] || [ '$ACTION' = 'boot' ]; then
            sudo /run/current-system/sw/bin/nix-env --profile /nix/var/nix/profiles/system --set \"\$SYS\"
          fi
          sudo \$SYS/bin/switch-to-configuration '$ACTION'
        "
      }

      do_switch() {
        require_vm "''${1:-}"
        local VM="$1"
        echo "nixling: building $VM..."
        local GEN
        GEN=$(do_build_inner "$VM")
        echo "nixling: syncing per-VM store..."
        sync_store "$VM" "$GEN"
        if ! vm_running "$VM" \
           && ! vm_active "$VM"; then
          echo "nixling: $VM is not running — staged the closure but skipping live activation."
          echo "  Start with 'nixling up $VM' to boot into the new generation."
          return 0
        fi
        echo "nixling: activating live in $VM..."
        vm_activate "$VM" switch
        echo "nixling: $VM switched to $GEN"
      }

      do_boot() {
        require_vm "''${1:-}"
        local VM="$1"
        echo "nixling: building $VM..."
        local GEN
        GEN=$(do_build_inner "$VM")
        echo "nixling: syncing per-VM store..."
        sync_store "$VM" "$GEN"
        if vm_running "$VM" \
           || vm_active "$VM"; then
          echo "nixling: bumping default-boot profile (no live activation)..."
          vm_activate "$VM" boot
        else
          echo "nixling: $VM is down; new closure will be the default at next start."
        fi
        echo "nixling: $VM boot-staged on $GEN"
      }

      do_test() {
        require_vm "''${1:-}"
        local VM="$1"
        if ! vm_running "$VM" \
           && ! vm_active "$VM"; then
          echo "nixling: 'test' requires a running VM (it activates live without bumping default boot)." >&2
          exit 2
        fi
        echo "nixling: building $VM..."
        local GEN
        GEN=$(do_build_inner "$VM")
        echo "nixling: syncing per-VM store..."
        sync_store "$VM" "$GEN"
        echo "nixling: activating live (test — default boot NOT bumped)..."
        vm_activate "$VM" test
        echo "nixling: $VM tested on $GEN (default-boot unchanged)"
      }

      do_rollback() {
        require_vm "''${1:-}"
        local VM="$1"
        if ! vm_running "$VM" \
           && ! vm_active "$VM"; then
          echo "nixling: 'rollback' requires a running VM." >&2
          exit 2
        fi
        echo "nixling: rolling back $VM..."
        vm_ssh "$VM" "
          set -euo pipefail
          sudo /run/current-system/sw/bin/nix-env --profile /nix/var/nix/profiles/system --rollback
          NEW=\$(readlink /nix/var/nix/profiles/system)
          sudo \$NEW/bin/switch-to-configuration switch
        "
        echo "nixling: $VM rolled back. (Host per-VM store unchanged; the previous"
        echo "         generation's closure was already retained by the sync helper.)"
      }

      do_generations() {
        require_vm "''${1:-}"
        local VM="$1"
        local META=$STATE_ROOT/$VM/store-meta
        echo "=== Host-side per-VM store generations ($META/generations) ==="
        if [ ! -d "$META/generations" ]; then
          echo "  (none yet — run 'nixling build $VM')"
        else
          local CUR=""
          [ -L "$META/current" ] && CUR=$(basename "$(readlink "$META/current")")
          local GENS
          GENS=$(find "$META/generations" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort -n)
          for g in $GENS; do
            case "$g" in
              '''|*[!0-9]*) continue ;;
            esac
            local marker="" sys ts
            [ "$g" = "$CUR" ] && marker=" (current)"
            sys=$(readlink "$META/generations/$g/system" 2>/dev/null || echo "?")
            ts=$(jq -r '.timestamp // 0' "$META/generations/$g/meta.json" 2>/dev/null || echo 0)
            local dt="?"
            [ "$ts" != "0" ] && dt=$(date -d "@$ts" +'%Y-%m-%d %H:%M:%S' 2>/dev/null || echo "?")
            printf "  %4s  %s  %s%s\n" "$g" "$dt" "$sys" "$marker"
          done
        fi
        echo
        echo "=== In-VM nix-profile generations ==="
        if vm_running "$VM" || vm_active "$VM"; then
          local USER_AT_IP KEY CRED
          CRED=$(require_ssh "$VM")
          USER_AT_IP=$(echo "$CRED" | awk '{print $1}')
          KEY=$(echo "$CRED" | awk '{print $2}')
          ssh -i "$KEY" \
              -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$STATE_ROOT/known_hosts.nixling" \
              -o ConnectTimeout=5 \
              "$USER_AT_IP" \
              "sudo /run/current-system/sw/bin/nix-env --profile /nix/var/nix/profiles/system --list-generations 2>/dev/null
               echo
               echo 'booted:  '\"\$(readlink /run/booted-system 2>/dev/null || echo '?')\"
               echo 'current: '\"\$(readlink /run/current-system 2>/dev/null || echo '?')\"" \
            || echo "  (VM unreachable)"
        else
          echo "  ($VM is not running — start it and try again)"
        fi
      }

      do_gc() {
        require_vm "''${1:-}"
        local VM="$1"
        # Re-run the sync helper against the current generation; this
        # re-applies the retention sweep without producing a new
        # generation (since the helper short-circuits when the
        # toplevel is unchanged from current). To force a sweep even
        # when current is up-to-date, point next-generation at the
        # current generation dir and run.
        local META=$STATE_ROOT/$VM/store-meta
        if [ ! -L "$META/current" ]; then
          echo "nixling: $VM has no per-VM store yet (run 'nixling build $VM' first)." >&2
          exit 2
        fi
        local CURGEN
        CURGEN=$(readlink -f "$META/current")
        echo "nixling: gc on $VM against $CURGEN"
        sync_store "$VM" "$CURGEN"
        # Re-running the sync helper short-circuits because the system
        # path matches the current. Force the retention sweep by
        # building a fresh closure (idempotent if config unchanged) and
        # re-syncing — that path always runs the retention pass.
        local GEN
        GEN=$(do_build_inner "$VM")
        if [ "$GEN" != "$CURGEN" ]; then
          # Caller-driven race: someone changed the config between
          # `gc` invocation and `nix build`. Fall through to a full
          # sync (which also retains the old running generation).
          echo "nixling: closure changed mid-gc — performing a full switch instead."
          sync_store "$VM" "$GEN"
        fi
        echo "nixling: gc done. Store size:"
        sudo -A du -sh "$STATE_ROOT/$VM/store" 2>/dev/null || true
      }

      do_trust() {
        require_vm "''${1:-}"
        local VM="$1"
        local STATIC_IP
        STATIC_IP=$(vm_get "$VM" staticIp)
        if [ "$STATIC_IP" = "null" ]; then
          echo "nixling: '$VM' has no staticIp - cannot trust." >&2
          exit 2
        fi
        local KNOWN_HOSTS="$STATE_ROOT/known_hosts.nixling"
        echo "nixling: scanning host key for $VM at $STATIC_IP ..."
        local KEYSCAN
        KEYSCAN=$(ssh-keyscan -t ed25519 "$STATIC_IP" 2>/dev/null)
        if [ -z "$KEYSCAN" ]; then
          echo "nixling: could not fetch host key from $STATIC_IP - is $VM running?" >&2
          exit 3
        fi
        # Serialise all known_hosts.nixling mutations with flock, build the
        # new file in a temp path, then atomically replace. KEYSCAN is passed
        # as a positional arg to avoid shell injection in the heredoc.
        sudo -A bash -s -- "$STATE_ROOT" "$KNOWN_HOSTS" "$STATIC_IP" "$KEYSCAN" <<${"'"}BASH${"'"}
          set -euo pipefail
          STATE_ROOT=$1; KNOWN_HOSTS=$2; STATIC_IP=$3; KEYSCAN=$4
          exec 9>"$STATE_ROOT/known_hosts.nixling.lock"
          flock -w 30 9 || { echo "nixling: could not acquire known_hosts lock" >&2; exit 5; }
          tmp=$(mktemp "$STATE_ROOT/known_hosts.nixling.XXXXXX")
          if [ -f "$KNOWN_HOSTS" ]; then
            cp "$KNOWN_HOSTS" "$tmp"
          fi
          ssh-keygen -R "$STATIC_IP" -f "$tmp" 2>/dev/null || true
          rm -f "$tmp.old"
          printf '%s\n' "$KEYSCAN" >> "$tmp"
          chown root:root "$tmp"
          chmod 0644 "$tmp"
          mv -fT "$tmp" "$KNOWN_HOSTS"
BASH

        echo "nixling: pinned $VM ($STATIC_IP) in $KNOWN_HOSTS"
      }

      do_rotate_known_host() {
        require_vm "''${1:-}"
        local VM="$1"
        local STATIC_IP
        STATIC_IP=$(vm_get "$VM" staticIp)
        if [ "$STATIC_IP" = "null" ]; then
          echo "nixling: '$VM' has no staticIp." >&2
          exit 2
        fi
        local KNOWN_HOSTS="$STATE_ROOT/known_hosts.nixling"
        if [ ! -f "$KNOWN_HOSTS" ]; then
          echo "nixling: $KNOWN_HOSTS does not exist - nothing to rotate." >&2
          exit 2
        fi
        # Serialise with flock; rebuild temp file excluding the VM's entry, then
        # atomically replace.
        sudo -A bash -s -- "$STATE_ROOT" "$KNOWN_HOSTS" "$STATIC_IP" <<${"'"}BASH${"'"}
          set -euo pipefail
          STATE_ROOT=$1; KNOWN_HOSTS=$2; STATIC_IP=$3
          exec 9>"$STATE_ROOT/known_hosts.nixling.lock"
          flock -w 30 9 || { echo "nixling: could not acquire known_hosts lock" >&2; exit 5; }
          tmp=$(mktemp "$STATE_ROOT/known_hosts.nixling.XXXXXX")
          cp "$KNOWN_HOSTS" "$tmp"
          ssh-keygen -R "$STATIC_IP" -f "$tmp" 2>/dev/null || true
          rm -f "$tmp.old"
          chown root:root "$tmp"
          chmod 0644 "$tmp"
          mv -fT "$tmp" "$KNOWN_HOSTS"
BASH

        echo "nixling: removed stale key for $VM ($STATIC_IP). Run 'nixling trust $VM' after the VM reboots."
      }

      # ──────────────────────────────────────────────────────────────────────
      # 'nixling keys' — list / show / rotate framework-managed SSH keys
      # ──────────────────────────────────────────────────────────────────────
      do_keys() {
        local SUBCMD="''${1:-}"
        case "$SUBCMD" in
          list)    shift; do_keys_list "$@" ;;
          show)    shift; do_keys_show "$@" ;;
          rotate)  shift; do_keys_rotate "$@" ;;
          -h|--help|"")
            cat <<EOF
nixling keys <subcommand>

Manage the per-VM SSH keys nixling generates and stores under
${config.nixling.site.keysDir}.

Subcommands:
  list [--json]      Show every declared VM's key fingerprint, path,
                     and on-disk age. Without --json, prints a
                     human-readable table.
  show <vm>          Print the public key for <vm>.
  rotate <vm>        Generate a fresh keypair for <vm>, retain the
                     previous key under .../old/<timestamp>/, push
                     the new pubkey into the running VM's
                     authorized_keys via SSH using the OLD key, then
                     verify the new key works before scrubbing the
                     OLD pubkey from authorized_keys (matched by
                     SHA256 fingerprint).
                     Retention: 3 most recent generations are kept
                     under .../old/; older generations are pruned
                     after a successful rotation.
EOF
            return 0
            ;;
          *)
            echo "nixling keys: unknown subcommand '$SUBCMD'" >&2
            return 2
            ;;
        esac
      }

      do_keys_list() {
        local JSON=false
        case "''${1:-}" in --json) JSON=true; shift ;; esac

        local keys_dir
        keys_dir=${lib.escapeShellArg config.nixling.site.keysDir}
        if [ ! -d "$keys_dir" ]; then
          echo "nixling keys: keys directory $keys_dir does not exist" >&2
          return 1
        fi

        local _json="[]"
        for vm in $(manifest_vms); do
          local priv="$keys_dir/$vm"_ed25519
          local pub="$priv.pub"
          local fp="" age="" path_status="missing"
          if [ -f "$pub" ]; then
            path_status="present"
            fp=$(ssh-keygen -lf "$pub" 2>/dev/null \
              | awk '{print $2}' || echo "")
            # `stat -c %Y` is seconds since epoch; format as ISO 8601
            # using the system's date(1).
            local mtime
            mtime=$(stat -c '%Y' "$pub" 2>/dev/null || echo 0)
            if [ "$mtime" -gt 0 ]; then
              age=$(date -d "@$mtime" -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo "")
            fi
          fi
          _json=$(echo "$_json" | jq \
            --arg vm "$vm" --arg fp "$fp" --arg path "$priv" \
            --arg age "$age" --arg status "$path_status" \
            '. + [{vm:$vm, fingerprint:$fp, path:$path, mtime:$age, status:$status}]')
        done

        if [ "$JSON" = "true" ]; then
          echo "$_json" | jq .
          return 0
        fi

        printf '%-24s %-9s %-72s %-21s\n' "VM" "STATUS" "FINGERPRINT" "MTIME"
        printf '%-24s %-9s %-72s %-21s\n' \
          "------------------------" \
          "---------" \
          "------------------------------------------------------------------------" \
          "---------------------"
        echo "$_json" | jq -r '.[] |
          [.vm, .status, (.fingerprint // ""), (.mtime // "")]
          | @tsv' \
          | while IFS=$'\t' read -r v s f m; do
              printf '%-24s %-9s %-72s %-21s\n' "$v" "$s" "$f" "$m"
            done
      }

      do_keys_show() {
        local VM="''${1:-}"
        if [ -z "$VM" ]; then
          echo "Usage: nixling keys show <vm>" >&2
          return 2
        fi
        assert_safe "$VM"
        vm_exists "$VM" || { echo "nixling: unknown VM '$VM'" >&2; return 1; }
        local pub
        pub=${lib.escapeShellArg config.nixling.site.keysDir}/"$VM"_ed25519.pub
        if [ ! -f "$pub" ]; then
          echo "nixling keys: no pubkey at $pub — has nixos-rebuild run yet?" >&2
          return 1
        fi
        cat "$pub"
      }

      do_keys_rotate() {
        local VM="''${1:-}"
        if [ -z "$VM" ]; then
          echo "Usage: nixling keys rotate <vm>" >&2
          return 2
        fi
        assert_safe "$VM"
        vm_exists "$VM" || { echo "nixling: unknown VM '$VM'" >&2; return 1; }

        local keys_dir priv pub ts old_dir
        keys_dir=${lib.escapeShellArg config.nixling.site.keysDir}
        priv="$keys_dir/$VM"_ed25519
        pub="$priv.pub"
        ts=$(date -u +'%Y%m%dT%H%M%SZ')
        old_dir="$keys_dir/old/$ts"

        if [ ! -f "$priv" ]; then
          echo "nixling keys rotate: no existing key at $priv (run nixos-rebuild switch first)" >&2
          return 1
        fi

        # Quick reachability probe with the OLD key before doing
        # anything destructive — if we can't ssh now, rotation will
        # just lock us out.
        local IP SSH_USER
        IP=$(vm_get "$VM" staticIp)
        SSH_USER=$(vm_get "$VM" sshUser)
        if [ "$IP" = "null" ] || [ -z "$IP" ] \
            || [ "$SSH_USER" = "null" ] || [ -z "$SSH_USER" ]; then
          echo "nixling keys rotate: $VM has no staticIp/sshUser in manifest — cannot verify rotation; aborting" >&2
          return 1
        fi

        if ! sudo -A ssh -i "$priv" \
              -o BatchMode=yes -o ConnectTimeout=5 \
              -o StrictHostKeyChecking=yes \
              -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
              "$SSH_USER@$IP" true 2>/dev/null; then
          echo "nixling keys rotate: cannot SSH to $VM with the current key — aborting (try 'nixling status $VM' first)" >&2
          return 1
        fi

        # Capture old-key fingerprint BEFORE moving anything so we
        # can remove the matching line from the guest's
        # authorized_keys after the new key is verified. ssh-keygen
        # `-l` prints "<bits> SHA256:<hash> comment (type)". We grep
        # the SHA256:<hash> form because it is robust against
        # whitespace/encoding differences when matched against the
        # public-key line itself.
        local old_fp
        old_fp=$(sudo -A ssh-keygen -l -f "$pub" 2>/dev/null \
          | awk '{print $2}')
        if [ -z "$old_fp" ]; then
          echo "nixling keys rotate: could not read old-key fingerprint from $pub — aborting" >&2
          return 1
        fi

        # Stash old key.
        sudo -A install -d -m 0700 -o root -g root "$old_dir"
        sudo -A install -m 0640 -o root -g root "$priv" "$old_dir/$VM"_ed25519
        sudo -A install -m 0644 -o root -g root "$pub"  "$old_dir/$VM"_ed25519.pub
        echo "nixling keys rotate: stashed previous key under $old_dir"

        # Generate fresh key in place (atomic via .new + mv -T).
        local new_priv new_pub
        new_priv="$priv.new.$$"
        new_pub="$new_priv.pub"
        sudo -A rm -f "$new_priv" "$new_pub"
        if ! sudo -A ssh-keygen -t ed25519 -N "" \
              -C "nixling:$VM (rotated $ts)" \
              -f "$new_priv" >/dev/null; then
          echo "nixling keys rotate: ssh-keygen failed — aborting" >&2
          sudo -A rm -f "$new_priv" "$new_pub"
          return 1
        fi
        sudo -A chmod 0640 "$new_priv"
        sudo -A chmod 0644 "$new_pub"
        sudo -A chown root:root "$new_priv" "$new_pub"
        sudo -A setfacl -m "g:nixling-launcher:r" "$new_priv" || true

        # Append the new pubkey to the VM's authorized_keys using the
        # OLD key. We don't remove the old key yet — that happens
        # AFTER the new key is verified.
        local new_pub_content
        new_pub_content=$(sudo -A cat "$new_pub")
        if ! sudo -A ssh -i "$priv" \
              -o BatchMode=yes -o ConnectTimeout=5 \
              -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
              "$SSH_USER@$IP" \
              "umask 077; mkdir -p ~/.ssh; printf '%s\n' '$new_pub_content' >> ~/.ssh/authorized_keys"; then
          echo "nixling keys rotate: failed to push new pubkey via SSH — aborting" >&2
          sudo -A rm -f "$new_priv" "$new_pub"
          return 1
        fi

        # Verify the new key works.
        if ! sudo -A ssh -i "$new_priv" \
              -o BatchMode=yes -o ConnectTimeout=5 \
              -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
              "$SSH_USER@$IP" true 2>/dev/null; then
          echo "nixling keys rotate: NEW key cannot reach $VM; OLD key still in place — aborting" >&2
          echo "  to recover manually: re-run nixos-rebuild switch (re-installs the framework-managed key in the next boot) OR" >&2
          echo "  ssh in with the OLD key (still at $old_dir) and inspect ~/.ssh/authorized_keys" >&2
          sudo -A rm -f "$new_priv" "$new_pub"
          return 1
        fi

        # Activate: swap in the new key files.
        sudo -A mv -fT "$new_priv" "$priv"
        sudo -A mv -fT "$new_pub" "$pub"

        # W3b H5: remove the OLD pubkey from the guest's
        # authorized_keys using the NEW key. We match on the
        # ssh-keygen fingerprint (SHA256:<hash>); any line whose
        # `ssh-keygen -l` fingerprint matches `$old_fp` is dropped.
        # Failure here is non-fatal — the rotation already succeeded
        # and the guest still rejects every key except the new one
        # would be a property of authorized_keys, not of access. But
        # we WARN so the operator can clean up manually.
        if ! sudo -A ssh -i "$priv" \
              -o BatchMode=yes -o ConnectTimeout=10 \
              -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
              "$SSH_USER@$IP" \
              "set -e
              ak=\$HOME/.ssh/authorized_keys
              [ -f \"\$ak\" ] || exit 0
              tmp=\$(mktemp \"\$ak.rotate.XXXXXX\")
              while IFS= read -r line; do
                # Empty or pure-comment lines pass through.
                case \"\$line\" in
                  \"\"|\\#*)
                    printf '%s\n' \"\$line\" >> \"\$tmp\"
                    continue
                    ;;
                esac
                fp=\$(printf '%s\n' \"\$line\" | ssh-keygen -l -f - 2>/dev/null | awk '{print \$2}')
                if [ \"\$fp\" != \"$old_fp\" ]; then
                  printf '%s\n' \"\$line\" >> \"\$tmp\"
                fi
              done < \"\$ak\"
              chmod 0600 \"\$tmp\"
              mv -f \"\$tmp\" \"\$ak\"" 2>/dev/null; then
          echo "nixling keys rotate: WARN failed to scrub OLD pubkey ($old_fp) from $VM authorized_keys" >&2
          echo "  log in with the NEW key and manually remove the matching line if you want to revoke the OLD key." >&2
        else
          echo "nixling keys rotate: removed OLD pubkey ($old_fp) from $VM authorized_keys"
        fi

        # W3b H5: bounded retention. Keep the 3 most recent
        # generations under $keys_dir/old/; delete anything older.
        # `ls -1t` sorts newest-first; tail -n +4 drops the first 3.
        # The directory is root-owned 0700 so the operator can still
        # inspect / move out any generation manually before pruning
        # if needed.
        local _gc_target
        if [ -d "$keys_dir/old" ]; then
          while IFS= read -r _gc_target; do
            [ -n "$_gc_target" ] || continue
            echo "nixling keys rotate: pruning old generation $_gc_target"
            sudo -A rm -rf -- "$keys_dir/old/$_gc_target"
          done < <(sudo -A ls -1t "$keys_dir/old" 2>/dev/null | tail -n +4)
        fi

        echo "nixling keys rotate: $VM rotated successfully."
        echo "  retention: 3 most recent generations kept under $keys_dir/old/"
        echo "  run 'nixos-rebuild switch' to refresh the host-keys share so future"
        echo "    VM boots install the new key automatically."
      }

      # ──────────────────────────────────────────────────────────────────────
      # 'nixling audit' — §4.5 security-posture JSON report
      # ──────────────────────────────────────────────────────────────────────
      do_audit() {
        local STRICT=false HUMAN=false
        while [ $# -gt 0 ]; do
          case "$1" in
            --strict) STRICT=true;  shift ;;
            --human)  HUMAN=true;   shift ;;
            -h|--help)
              printf 'nixling audit [--strict] [--human]\n'
              printf '  Emit a JSON security-posture report (§4.5 of SECURITY-nixling.md).\n'
              printf '  --strict exits non-zero if any field deviates from post-hardening target.\n'
              printf '  --human (or tty stdout) formats for humans.\n'
              exit 0 ;;
            -*) echo "nixling audit: unknown flag '$1'" >&2; exit 2 ;;
            *)  echo "nixling audit: unexpected argument '$1'" >&2; exit 2 ;;
          esac
        done
        [ -t 1 ] && [ "$STRICT" = "false" ] && HUMAN=true

        local _deviations=0
        _audit_fail() { echo "STRICT-FAIL: $1 (expected $2, got $3)" >&2; _deviations=$((_deviations + 1)); }

        # ── kvm_dev_mode ────────────────────────────────────────────────────
        local _kvm_mode
        _kvm_mode="''${NIXLING_AUDIT_TESTMODE_KVM_MODE:-$(stat -c '%a' /dev/kvm 2>/dev/null || echo "missing")}"

        # ── wayland_user_in_kvm ─────────────────────────────────────────────
        # Audit: the host's configured Wayland user MUST NOT be in the
        # kvm group — only the per-VM `nixling-<vm>-gpu` sidecar users
        # and the microvm service user belong there. If
        # `nixling.site.waylandUser` is null (headless deployment), the
        # check is vacuously satisfied.
        local _wayland_user=${if config.nixling.site.waylandUser == null then ''""'' else ''"${config.nixling.site.waylandUser}"''}
        local _wayland_user_in_kvm=false
        if [ -n "$_wayland_user" ]; then
          id "$_wayland_user" >/dev/null 2>&1 \
            && id "$_wayland_user" 2>/dev/null | grep -qw kvm \
            && _wayland_user_in_kvm=true || true
        fi

        # ── store_delivery (from manifest + live virtiofsd service check) ───
        # P6r2 nixos-r2-1: use --value to get raw LoadState; non-net VMs
        # without a loaded virtiofsd unit report UNKNOWN (regression signal).
        local _store_json="{}"
        local _vm
        for _vm in $(manifest_vms); do
          local _svc_load _mode
          _svc_load=$(systemctl show "microvm-virtiofsd@$_vm.service" \
            -p LoadState --value 2>/dev/null || echo "not-found")
          if [ "$_svc_load" = "loaded" ]; then
            _mode="virtiofs"
          else
            local _is_net_vm
            _is_net_vm=$(vm_get "$_vm" isNetVm)
            if [ "$_is_net_vm" = "true" ]; then
              _mode="erofs"
            else
              _mode="UNKNOWN"
            fi
          fi
          _store_json=$(echo "$_store_json" | jq --arg k "$_vm" --arg v "$_mode" '. + {($k): $v}')
        done

        # ── virtiofsd (per running workload VM) ──────────────────────────────
        local _vfsd_json="{}"
        for _vm in $(manifest_vms); do
          [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
          local _pid=""
          _pid=$(pgrep -af 'virtiofsd' 2>/dev/null \
                 | grep -v supervisord \
                 | grep -- "$_vm" \
                 | head -n1 | awk '{print $1}') || true
          [ -z "$_pid" ] && continue

          local _uid _user
          _uid=$(awk '/^Uid:/ {print $2; exit}' "/proc/$_pid/status" 2>/dev/null || echo "0")
          _user=$(id -nu "$_uid" 2>/dev/null || echo "uid=$_uid")

          local _cap_eff _last2 _last_byte _dac_dropped=false _caps_json
          _cap_eff=$(awk '/^CapEff:/ {print $2; exit}' "/proc/$_pid/status" 2>/dev/null \
                     || echo "0000000000000000")
          _last2=$(printf '%s' "$_cap_eff" | tail -c 2)
          _last_byte=$(printf '%d' "0x$_last2" 2>/dev/null || echo "255")
          if (( (_last_byte & 4) == 0 )); then
            _dac_dropped=true
          fi
          $_dac_dropped && _caps_json='["CAP_DAC_READ_SEARCH"]' || _caps_json='[]'

          local _ro=false _marker=false
          tr '\0' ' ' < "/proc/$_pid/cmdline" 2>/dev/null | grep -q -- '--readonly' \
            && _ro=true || true
          if [ -e "$STATE_ROOT/$_vm/store/.nixling-marker-$_vm" ] 2>/dev/null; then
            _marker=true
          fi

          local _entry
          _entry=$(jq -n \
            --arg u "$_user" \
            --argjson c "$_caps_json" \
            --argjson r "$_ro" \
            --argjson m "$_marker" \
            '{user: $u, caps_dropped: $c, readonly_flag: $r, marker_ok: $m}')
          _vfsd_json=$(echo "$_vfsd_json" | jq --arg k "$_vm" --argjson v "$_entry" '. + {($k): $v}')
        done

        # ── ssh PasswordAuthentication (host + workload VMs via nix eval) ───
        # security-r8-audio-5: invoking `sudo -A sshd -T` from inside a
        # systemd oneshot (nixling-audit-check.service, User=root) fails
        # without a terminal/ASKPASS because sudo cannot escalate to a
        # process group; the call swallowed all output via 2>/dev/null
        # and the audit then STRICT-FAILed with `null`. Use sudo only
        # when needed (sshd -T reads sshd_config which is root:root
        # 0600, so we DO need root). When already root, skip sudo
        # entirely; otherwise keep `sudo -A`.
        local _ssh_json="{}"
        local _sshd_out _host_pw=null _sshd_cmd=""
        if [ "$(id -u)" = "0" ]; then
          _sshd_cmd="sshd"
        else
          _sshd_cmd="sudo -A sshd"
        fi
        # sshd -T probes the host's effective sshd config. The
        # `user=` / `host=` / `addr=` triplet is what sshd-T uses as
        # the Match-evaluation context — we pick a generic "test"
        # principal so the audit doesn't depend on the host's real
        # username being declared.
        _sshd_out=$($_sshd_cmd -T -C "user=test,host=nixos,addr=" 2>/dev/null) || true
        if [ -n "$_sshd_out" ]; then
          local _pw
          _pw=$(echo "$_sshd_out" | grep -i '^passwordauthentication' | awk '{print $2}' | head -1)
          [ "$_pw" = "no" ] && _host_pw=false || _host_pw=true
        fi
        _ssh_json=$(echo "$_ssh_json" | jq --argjson v "$_host_pw" \
          '. + {"host": {"PasswordAuthentication": $v}}')
        for _vm in $(manifest_vms); do
          [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
          local _vm_pw=null _nix_val="" _rc_nix=0
          # The flake path comes from `nixling.site.flakePath` (or the
          # NIXLING_FLAKE override). Falling back to the unset case
          # (empty FLAKE_DEFAULT) means we just skip this per-VM probe
          # — the audit's strict mode still passes if the host-side
          # PasswordAuthentication check above is fine.
          if [ -n "''${NIXLING_FLAKE:-$FLAKE_DEFAULT}" ]; then
            _nix_val=$(nix eval --raw \
              "''${NIXLING_FLAKE:-$FLAKE_DEFAULT}#nixosConfigurations.''${NIXLING_AUDIT_NIXOS_CONFIG:-nixos}.config.microvm.vms.$_vm.config.config.services.openssh.settings.PasswordAuthentication" \
              2>/dev/null) || _rc_nix=$?
            if [ "$_rc_nix" -eq 0 ] && [ -n "$_nix_val" ]; then
              [ "$_nix_val" = "false" ] && _vm_pw=false || _vm_pw=true
            fi
          fi
          _ssh_json=$(echo "$_ssh_json" | jq \
            --arg k "$_vm" --argjson v "$_vm_pw" \
            '. + {($k): {"PasswordAuthentication": $v}}')
        done

        # ── bridge_isolation (tap isolation via 'bridge link') ──────────────
        # P6r2 security-r2-1: keyed by VM (not bridge) so two workloads on
        # the same bridge cannot mask each other via last-write-wins.
        local _bridge_json="{}"
        for _vm in $(manifest_vms); do
          [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
          local _bridge _tap
          _bridge=$(vm_get "$_vm" bridge)
          _tap=$(vm_get "$_vm" tap)
          [ "$_bridge" = "null" ] || [ -z "$_bridge" ] && continue
          local _iso_state="unknown" _isolated=false
          if bridge -d link show "$_tap" 2>/dev/null | grep -q 'isolated on'; then
            _iso_state="isolated"
            _isolated=true
          elif bridge -d link show "$_tap" 2>/dev/null | grep -q 'isolated off'; then
            _iso_state="not-isolated"
          elif ! ip link show "$_tap" >/dev/null 2>&1; then
            _iso_state="tap-missing"
          fi
          _bridge_json=$(echo "$_bridge_json" | jq \
            --arg k "$_vm" --arg b "$_bridge" --arg t "$_tap" \
            --arg s "$_iso_state" --argjson iso "$_isolated" \
            '. + {($k): {bridge: $b, tap: $t, state: $s, isolated: $iso}}')
        done

        # ── autoUpgrade_commits_lock ─────────────────────────────────────────
        # The check is a consumer-config concern (does the consumer's
        # flake set --commit-lock-file on system.autoUpgrade.flags?) so
        # we look in the configured flake path; null FLAKE_DEFAULT means
        # the probe is skipped.
        local _aul=false
        if [ -n "''${NIXLING_FLAKE:-$FLAKE_DEFAULT}" ] \
           && grep -q -- '--commit-lock-file' "''${NIXLING_FLAKE:-$FLAKE_DEFAULT}/flake.nix" 2>/dev/null; then
          _aul=true
        fi

        # ── fail2ban_active ──────────────────────────────────────────────────
        local _f2b=false
        systemctl is-active --quiet fail2ban 2>/dev/null && _f2b=true || true

        # ── sidecars_per_vm ──────────────────────────────────────────────────
        local _sidecar_json="{}"
        for _vm in $(manifest_vms); do
          local _g _a _gpu=false _snd=false _gpu_user="none" _snd_user="none"
          _g=$(vm_get "$_vm" graphics)
          _a=$(vm_get "$_vm" audio)
          if [ "$_g" = "true" ] && systemctl is-active --quiet "nixling-$_vm-gpu.service" 2>/dev/null; then
            _gpu=true
            local _gpid
            _gpid=$(systemctl show "nixling-$_vm-gpu.service" -p MainPID --value 2>/dev/null || echo "0")
            [ "''${_gpid:-0}" != "0" ] && \
              _gpu_user=$(awk '/^Uid:/ {print $2; exit}' "/proc/$_gpid/status" 2>/dev/null \
                          | xargs id -nu 2>/dev/null || echo "uid=$_gpid") || true
          fi
          if [ "$_a" = "true" ] && systemctl is-active --quiet "nixling-$_vm-snd.service" 2>/dev/null; then
            _snd=true
            local _spid
            _spid=$(systemctl show "nixling-$_vm-snd.service" -p MainPID --value 2>/dev/null || echo "0")
            [ "''${_spid:-0}" != "0" ] && \
              _snd_user=$(awk '/^Uid:/ {print $2; exit}' "/proc/$_spid/status" 2>/dev/null \
                          | xargs id -nu 2>/dev/null || echo "uid=$_spid") || true
          fi
          local _se
          _se=$(jq -n --argjson g "$_gpu" --argjson s "$_snd" \
            --arg gu "$_gpu_user" --arg su "$_snd_user" \
            '{gpu_active: $g, snd_active: $s, gpu_user: $gu, snd_user: $su}')
          _sidecar_json=$(echo "$_sidecar_json" | jq \
            --arg k "$_vm" --argjson v "$_se" '. + {($k): $v}')
        done

        # ── usbipd_per_env_isolation ─────────────────────────────────────────
        local _usbipd_json="{}"
        local _env
        for _env in $(jq -r '[.[].env | select(. != null)] | unique[]' "$MANIFEST" 2>/dev/null \
                      || true); do
          local _ussocket=false _usbackend=false _ulk=false
          # socket is always-active when usbipd is configured for this env
          systemctl is-active --quiet "nixling-sys-$_env-usbipd-proxy.socket" 2>/dev/null \
            && _ussocket=true || true
          # backend service is on-demand (active only while USB session running)
          systemctl is-active --quiet "nixling-sys-$_env-usbipd-backend.service" 2>/dev/null \
            && _usbackend=true || true
          [ -e "/run/nixling/usbipd.lock" ] && _ulk=true || true
          local _ue
          _ue=$(jq -n --argjson p "$_ussocket" --argjson b "$_usbackend" --argjson l "$_ulk" \
            '{socket_active: $p, backend_active: $b, lock_present: $l}')
          _usbipd_json=$(echo "$_usbipd_json" | jq \
            --arg k "$_env" --argjson v "$_ue" '. + {($k): $v}')
        done

        # ── Nix-baked constants (evaluated at build / NixOS-generation time) ─
        local _ch_ver="${auditChVersion}"
        local _crosvm_rev="${auditCrosvmRev}"
        local _seccomp_rev="${auditSeccompRev}"
        local _ch_pair_ok=${auditChCrosvmPairOk}

        # ── Assemble report JSON ─────────────────────────────────────────────
        local _report
        _report=$(jq -n \
          --arg kvm_dev_mode "$_kvm_mode" \
          --argjson wayland_user_in_kvm "$_wayland_user_in_kvm" \
          --argjson store_delivery "$_store_json" \
          --argjson virtiofsd "$_vfsd_json" \
          --argjson ssh "$_ssh_json" \
          --argjson bridge_isolation "$_bridge_json" \
          --argjson autoUpgrade_commits_lock "$_aul" \
          --arg ch_version "$_ch_ver" \
          --arg crosvm_rev "$_crosvm_rev" \
          --arg seccomp_rev "$_seccomp_rev" \
          --argjson ch_crosvm_pair_ok "$_ch_pair_ok" \
          --argjson fail2ban_active "$_f2b" \
          --argjson sidecars_per_vm "$_sidecar_json" \
          --argjson usbipd_per_env_isolation "$_usbipd_json" \
          '{
            kvm_dev_mode: $kvm_dev_mode,
            wayland_user_in_kvm: $wayland_user_in_kvm,
            store_delivery: $store_delivery,
            virtiofsd: $virtiofsd,
            ssh: $ssh,
            bridge_isolation: $bridge_isolation,
            autoUpgrade_commits_lock: $autoUpgrade_commits_lock,
            ch_version: $ch_version,
            crosvm_rev: $crosvm_rev,
            seccomp_rev: $seccomp_rev,
            ch_crosvm_pair_ok: $ch_crosvm_pair_ok,
            fail2ban_active: $fail2ban_active,
            sidecars_per_vm: $sidecars_per_vm,
            usbipd_per_env_isolation: $usbipd_per_env_isolation
          }')

        # ── Strict checks ────────────────────────────────────────────────────
        if [ "$STRICT" = "true" ]; then
          [ "$_kvm_mode" = "660" ] \
            || _audit_fail "kvm_dev_mode" "660" "$_kvm_mode"
          [ "$_wayland_user_in_kvm" = "false" ] \
            || _audit_fail "wayland_user_in_kvm" "false" "$_wayland_user_in_kvm"

          for _vm in $(manifest_vms); do
            [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
            local _ve
            _ve=$(echo "$_vfsd_json" | jq -r --arg v "$_vm" '.[$v]')
            [ "$_ve" = "null" ] \
              && echo "AUDIT SKIP [virtiofsd.$_vm]: daemon not running" >&2 \
              && continue
            local _vro _vmk _vnd
            _vro=$(echo "$_ve" | jq -r '.readonly_flag')
            _vmk=$(echo "$_ve" | jq -r '.marker_ok')
            _vnd=$(echo "$_ve" | jq '.caps_dropped | length')
            [ "$_vro" = "true" ] \
              || _audit_fail "virtiofsd.$_vm.readonly_flag" "true" "$_vro"
            [ "$_vmk" = "true" ] \
              || _audit_fail "virtiofsd.$_vm.marker_ok" "true" "$_vmk"
            [ "$_vnd" -ge 1 ] \
              || _audit_fail "virtiofsd.$_vm.caps_dropped" ">=1 dropped" "0"
          done

          local _hpw
          _hpw=$(echo "$_report" | jq -r '.ssh.host.PasswordAuthentication')
          [ "$_hpw" = "false" ] \
            || _audit_fail "ssh.host.PasswordAuthentication" "false" "$_hpw"

          [ "$_aul" = "false" ] \
            || _audit_fail "autoUpgrade_commits_lock" "false" "$_aul"
          [ "$_ch_pair_ok" = "true" ] \
            || _audit_fail "ch_crosvm_pair_ok" "true" "$_ch_pair_ok"
          [ "$_f2b" = "true" ] \
            || _audit_fail "fail2ban_active" "true" "$_f2b"

          # ── Strict: store_delivery ───────────────────────────────────────
          # P6r1 nixos-1: workloads MUST use virtiofs.
          # C0 net-VM→erofs DEFERRED; skip net-VM store-delivery check.
          for _vm in $(manifest_vms); do
            [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
            local _sd_val
            _sd_val=$(echo "$_store_json" | jq -r --arg v "$_vm" '.[$v]')
            [ "$_sd_val" = "virtiofs" ] \
              || _audit_fail "store_delivery.$_vm" "virtiofs" "$_sd_val"
          done

          # ── Strict: per-VM ssh.PasswordAuthentication ─────────────────────
          # P6r1 security-1: per-VM PasswordAuthentication MUST be false.
          for _vm in $(manifest_vms); do
            [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
            local _vpw
            _vpw=$(echo "$_ssh_json" | jq -r --arg v "$_vm" '.[$v].PasswordAuthentication')
            [ "$_vpw" = "null" ] && continue
            [ "$_vpw" = "false" ] \
              || _audit_fail "ssh.$_vm.PasswordAuthentication" "false" "$_vpw"
          done

          # ── Strict: bridge_isolated_workload ──────────────────────────────
          # P6r2 security-r2-1: keyed by VM; assert every workload has isolated==true.
          # v0.1.4 fix: skip when the VM isn't running. The bridge-isolation
          # property is a runtime attribute of the workload's tap; a stopped
          # VM has no tap, so jq returns null. Mirrors the
          # `AUDIT SKIP [virtiofsd.<vm>]: daemon not running` semantic
          # already applied to the virtiofsd check above.
          for _vm in $(manifest_vms); do
            [ "$(vm_get "$_vm" isNetVm)" = "true" ] && continue
            local _bridge
            _bridge=$(vm_get "$_vm" bridge)
            [ "$_bridge" = "null" ] || [ -z "$_bridge" ] && continue
            # Skip stopped VMs: the tap doesn't exist on the bridge.
            # v0.1.6: graphics VMs run cloud-hypervisor via the
            # nixling-<vm>-gpu sidecar, NOT microvm@<vm>.service (the
            # GPU sidecar replaces the upstream runner). Pre-v0.1.6
            # the audit only checked microvm@<vm>, which made it
            # blanket-skip all graphics VMs even when they were
            # running. Now: a VM is "running" if any of nixling@<vm>,
            # microvm@<vm>, or nixling-<vm>-gpu is active.
            local _vm_active=inactive
            if systemctl is-active --quiet "nixling@''${_vm}.service" 2>/dev/null \
               || systemctl is-active --quiet "microvm@''${_vm}.service" 2>/dev/null \
               || systemctl is-active --quiet "nixling-''${_vm}-gpu.service" 2>/dev/null; then
              _vm_active=active
            fi
            if [ "$_vm_active" != "active" ]; then
              echo "AUDIT SKIP [bridge_isolated_workload.$_vm]: VM not running" >&2
              continue
            fi
            local _biso
            _biso=$(echo "$_bridge_json" | jq -r --arg v "$_vm" '.[$v].isolated')
            [ "$_biso" = "true" ] \
              || _audit_fail "bridge_isolated_workload.$_vm" "isolated:true" "$_biso"
          done

          # ── Strict: sidecars_per_vm dedicated user ────────────────────────
          # P6r1 security-1: gpu/snd sidecars MUST run as nixling-<vm>-gpu
          # and nixling-<vm>-snd, NOT the host user.
          for _vm in $(manifest_vms); do
            local _sc_entry
            _sc_entry=$(echo "$_sidecar_json" | jq -r --arg v "$_vm" '.[$v]')
            local _sc_gpu _sc_snd _sc_gpu_user _sc_snd_user
            _sc_gpu=$(echo "$_sc_entry" | jq -r '.gpu_active')
            _sc_snd=$(echo "$_sc_entry" | jq -r '.snd_active')
            _sc_gpu_user=$(echo "$_sc_entry" | jq -r '.gpu_user')
            _sc_snd_user=$(echo "$_sc_entry" | jq -r '.snd_user')
            if [ "$_sc_gpu" = "true" ]; then
              [ "$_sc_gpu_user" = "nixling-$_vm-gpu" ] \
                || _audit_fail "sidecars_per_vm.$_vm.gpu_user" "nixling-$_vm-gpu" "$_sc_gpu_user"
            fi
            if [ "$_sc_snd" = "true" ]; then
              [ "$_sc_snd_user" = "nixling-$_vm-snd" ] \
                || _audit_fail "sidecars_per_vm.$_vm.snd_user" "nixling-$_vm-snd" "$_sc_snd_user"
            fi
          done

          # ── Strict: usbipd_per_env_isolation ──────────────────────────────
          # P6r1 security-2: usbipd socket MUST be active per env (proves
          # per-env isolation is configured; backend is on-demand).
          for _env in $(jq -r '[.[].env | select(. != null)] | unique[]' "$MANIFEST" 2>/dev/null \
                        || true); do
            local _u_socket
            _u_socket=$(echo "$_usbipd_json" | jq -r --arg e "$_env" '.[$e].socket_active')
            [ "$_u_socket" = "true" ] \
              || _audit_fail "usbipd_per_env_isolation.$_env.socket" "true" "$_u_socket"
          done
        fi

        # ── Output ───────────────────────────────────────────────────────────
        if [ "$HUMAN" = "true" ]; then
          printf '\n=== nixling security audit ===\n\n'
          printf '  %-40s %s\n' "kvm_dev_mode:"   \
            "$_kvm_mode $([ "$_kvm_mode" = "660" ] && echo '✓' || echo '✗')"
          printf '  %-40s %s\n' "wayland_user_in_kvm:"  \
            "$_wayland_user_in_kvm $([ "$_wayland_user_in_kvm" = "false" ] && echo '✓' || echo '✗ (must be false)')"
          printf '\n  store_delivery:\n'
          echo "$_store_json"  | jq -r 'to_entries[] | "    \(.key): \(.value)"'
          printf '\n  virtiofsd:\n'
          if [ "$(echo "$_vfsd_json" | jq 'length')" -eq 0 ]; then
            echo "    (no virtiofsd processes running)"
          else
            echo "$_vfsd_json" | jq -r \
              'to_entries[] | "    \(.key): user=\(.value.user) ro=\(.value.readonly_flag) marker=\(.value.marker_ok) caps_dropped=\(.value.caps_dropped|length)"'
          fi
          printf '\n  ssh:\n'
          echo "$_ssh_json" | jq -r \
            'to_entries[] | "    \(.key): PasswordAuthentication=\(.value.PasswordAuthentication)"'
          printf '\n  bridge_isolation:\n'
          if [ "$(echo "$_bridge_json" | jq 'length')" -eq 0 ]; then
            echo "    (no workload taps found or VMs not running)"
          else
            echo "$_bridge_json" | jq -r \
              'to_entries[] | "    \(.key): bridge=\(.value.bridge) tap=\(.value.tap) isolated=\(.value.isolated)"'
          fi
          printf '\n'
          printf '  %-40s %s\n' "autoUpgrade_commits_lock:" "$_aul"
          printf '  %-40s %s\n' "ch_version:"    "$_ch_ver"
          printf '  %-40s %s\n' "crosvm_rev:"    "$(printf '%s' "$_crosvm_rev" | cut -c1-12)…"
          printf '  %-40s %s\n' "seccomp_rev:"   "$(printf '%s' "$_seccomp_rev" | cut -c1-12)…"
          printf '  %-40s %s\n' "ch_crosvm_pair_ok:"  "$_ch_pair_ok"
          printf '  %-40s %s\n' "fail2ban_active:"    "$_f2b"
          printf '\n'
          if [ "$STRICT" = "true" ]; then
            if [ "$_deviations" -eq 0 ]; then
              printf '=== STRICT: all checks PASS ===\n'
            else
              printf '=== STRICT: %d deviation(s) found ===\n' "$_deviations" >&2
            fi
          fi
        else
          echo "$_report"
        fi

        if [ "$STRICT" = "true" ] && [ "$_deviations" -gt 0 ]; then
          exit 1
        fi
      }


            case "''${1:-}" in
        list)    shift; do_list ;;
        up)
          shift
          # `nixling up <vm> [-d|--detach]` (flag may also appear
          # before <vm>) — -d disowns the CH process and suppresses
          # the EXIT cleanup, so the VM keeps running after this
          # wrapper exits. Useful when launching from a long-lived
          # process (e.g. an agent shell) that might get reaped, or
          # when you just want to fire-and-forget.
          _DETACH=false
          _VM=""
          while [ $# -gt 0 ]; do
            case "$1" in
              -d|--detach) _DETACH=true; shift ;;
              --)          shift
                           if [ $# -gt 0 ]; then
                             if [ -z "$_VM" ]; then _VM="$1"; shift
                             else echo "nixling up: unexpected extra argument '$1'" >&2; exit 2
                             fi
                           fi
                           break ;;
              -*)          echo "nixling up: unknown flag '$1'" >&2; exit 2 ;;
              *)           if [ -z "$_VM" ]; then _VM="$1"; shift
                           else echo "nixling up: unexpected extra argument '$1'" >&2; exit 2
                           fi ;;
            esac
          done
          do_up "$_VM" "$_DETACH"
          ;;
        down)    shift; do_down "$@" ;;
        restart) shift; do_restart "$@" ;;
        status)  shift; do_status "$@" ;;
        usb)     shift; do_usb "$@" ;;
        console) shift; do_console "$@" ;;
        audio)   shift; do_audio "$@" ;;
        build)        shift; do_build "$@" ;;
        switch)       shift; do_switch "$@" ;;
        boot)         shift; do_boot "$@" ;;
        test)         shift; do_test "$@" ;;
        rollback)     shift; do_rollback "$@" ;;
        generations)  shift; do_generations "$@" ;;
        gc)           shift; do_gc "$@" ;;
        trust)             shift; do_trust "''${1:-}" ;;
        rotate-known-host) shift; do_rotate_known_host "''${1:-}" ;;
        keys)    shift; do_keys "$@" ;;
        audit)   shift; do_audit "$@" ;;
        -h|--help|help|"") usage 0 ;;
        *)
          echo "nixling: unknown subcommand '$1'" >&2
          usage 2
          ;;
      esac
    '';
  };

  # security-r8-audio-8 / -10: the launcher boots the VM AND spawns
  # a HOST-side terminal that SSHes in. The in-guest foot-autostart
  # service has been removed (see graphics.nix).
  #
  # We use KDE Konsole instead of foot. WHY:
  #   - foot relies on xdg-decoration for server-side decorations and
  #     does not implement client-side decorations. On Plasma 6 the
  #     SSD negotiation through Plasma's kwin is unreliable in some
  #     configurations (maintainer's setup: chromeless transparent
  #     window, no title bar, no buttons). Inside the guest the
  #     wayland-proxy-virtwl relay strips the decoration protocol
  #     so foot is always chromeless there.
  #   - Konsole is Plasma's first-class terminal: it always renders
  #     with full chrome on Plasma Wayland and X11, has built-in
  #     window-rule integration, and a sane tab/close-confirmation
  #     UX.
  #
  # Konsole's --qwindowtitle sets the window title; the WM_CLASS is
  # always "org.kde.konsole" (Konsole doesn't expose a CLI --app-id
  # flag), so the per-VM .desktop file's StartupWMClass below is set
  # to "org.kde.konsole" — and to keep per-VM identity in the
  # taskbar/title, the title carries the VM name (Plasma's "Special
  # Application Settings" or "Window Rules" can target it for icon
  # mapping if needed). The Plasma taskbar already groups by title
  # so distinguishing two VM consoles is easy.
  #
  # Wrapper script:
  #   1. `nixling up <vm> -d` boots the VM (idempotent — sec-r8-9).
  #   2. Waits up to 30s for sshd on <staticIp>:22.
  #   3. exec konsole + ssh with per-VM title.
  vmLaunchScript = name: vm:
    let
      meta      = manifest.${name};
      sshUser   = meta.sshUser;
      # W4-followup H2: resolve locally; the manifest no longer
      # carries `sshKeyPath` (see resolveSshKeyPath at the top of
      # this file).
      sshKey    = resolveSshKeyPath name vm;
      ip        = meta.staticIp;
    in
    if sshUser == null || sshKey == null || ip == null
    then
      # Missing SSH credentials in the manifest → fall back to just
      # booting the VM. The taskbar entry won't auto-launch a
      # terminal but `nixling up` still does its job. (No graphics
      # VM in production matches this case; this branch is the safe
      # fallback for partially-configured manifests.)
      pkgs.writeShellScript "nixling-launch-${name}-noterm" ''
        exec ${nixling}/bin/nixling up ${lib.escapeShellArg name} -d
      ''
    else
      pkgs.writeShellScript "nixling-launch-${name}" ''
        set -eu
        VM=${lib.escapeShellArg name}
        IP=${lib.escapeShellArg ip}
        SSH_USER=${lib.escapeShellArg sshUser}
        SSH_KEY=${lib.escapeShellArg sshKey}

        # 1. Bring the VM up (no-op if already running; sec-r8-9).
        ${nixling}/bin/nixling up "$VM" -d || {
          ${pkgs.libnotify}/bin/notify-send -i computer-fail \
            "nixling: failed to start $VM" \
            "See: journalctl -u nixling-$VM-gpu.service" \
            2>/dev/null || true
          exit 1
        }

        # 2. Wait for SSH to come up. The pinned host key in
        #    /var/lib/nixling/known_hosts.nixling is refreshed by
        #    nixling-known-hosts-refresh@<vm>.service, which runs
        #    automatically after the VM boots. As of
        #    security-r8-audio-12 that service auto-rotates the pin
        #    when the VM generation has changed, so we don't need
        #    any in-launcher key handling here.
        for _ in $(${pkgs.coreutils}/bin/seq 1 60); do
          if ${pkgs.openssh}/bin/ssh \
               -o BatchMode=yes \
               -o ConnectTimeout=2 \
               -o StrictHostKeyChecking=yes \
               -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
               -i "$SSH_KEY" "$SSH_USER@$IP" : 2>/dev/null; then
            break
          fi
          ${pkgs.coreutils}/bin/sleep 0.5
        done

        # 3. exec into a chrome'd host Konsole that SSHes in.
        #    --hide-menubar / --hide-tabbar: keep the window clean;
        #    we don't need konsole's session management here.
        #    --notransparency: opaque background.
        #    Title: Konsole shows the running command's titlebar
        #    string (e.g. "alice@dev-vm: ~"), which is good enough
        #    for per-VM identity. Silence the harmless multimedia
        #    pipewire warnings via QT_LOGGING_RULES.
        export QT_LOGGING_RULES="qt.multimedia.symbolsresolver.warning=false"
        exec ${pkgs.kdePackages.konsole}/bin/konsole \
          --hide-menubar \
          --hide-tabbar \
          --notransparency \
          -e ${pkgs.openssh}/bin/ssh \
            -o StrictHostKeyChecking=yes \
            -o UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling \
            -i "$SSH_KEY" "$SSH_USER@$IP"
      '';

  # Auto-generated .desktop entry per graphics-enabled VM. Konsole's
  # Wayland app_id is "org.kde.konsole" (fixed by the binary); the
  # title carries the per-VM identity. StartupWMClass therefore
  # matches Konsole's app_id rather than the VM name.
  #
  # Keywords + categories make the launcher discoverable from KRunner
  # (Alt+Space → type the VM name) and the Plasma application menu
  # (under both "System" and "Development", and as a generic
  # remote-access tool via "Network → RemoteAccess").
  desktopItems = lib.mapAttrsToList
    (name: vm: pkgs.makeDesktopItem {
      name = name;
      desktopName = "${name} VM";
      genericName = "nixling microVM";
      comment = "Start the ${name} microVM (if needed) and open an SSH session in Konsole";
      exec = "${vmLaunchScript name vm}";
      icon = "utilities-terminal";
      terminal = false;
      categories = [ "System" "Network" "RemoteAccess" "Development" ];
      keywords = [
        "nixling"
        "microVM"
        "VM"
        "ssh"
        "konsole"
        "terminal"
        name
      ];
      startupWMClass = "org.kde.konsole";
    })
    (lib.filterAttrs (_: vm: vm.graphics.enable) enabledVms);
in

{
  nixling.audioStateHelperPath = "${nl.nixlingReadAudioState}";
  nixling.cliBin = "${nixling}/bin/nixling";
  # nixling-vms-manifest is added by nixos-modules/manifest.nix.
  environment.systemPackages =
    [ nixling ] ++ desktopItems;
}
