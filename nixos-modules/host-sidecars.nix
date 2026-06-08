{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;
  otelAclRefreshCommand = "/run/current-system/sw/bin/nixling-otel-acl-refresh";

  # Wayland user's UID — used by the GPU sidecar to find the host
  # compositor's wayland-0 socket and ACL-grant the sidecar user. We
  # only compute it for graphics VMs (filtered below), and the
  # assertions module guarantees waylandUser is non-null whenever
  # graphics or audio VMs are declared. The `or null` default keeps
  # eval lazy on hosts with no graphics VMs declared.
  waylandUid =
    if cfg.site.waylandUser != null
    then toString (config.users.users.${cfg.site.waylandUser}.uid or 0)
    else "0";
in
{
  # ---------------------------------------------------------------------------
  # P4 C3: nixling-<vm>-swtpm.service — per-VM software-TPM emulator.
  #
  # H6: swtpm — per-VM dedicated static system user (was DynamicUser=kvm).
  # nixling-<vm>-swtpm:nixling-<vm>-swtpm mode 0600 socket; nixling-<vm>-gpu
  # gets rw via ACL. This prevents any kvm-group process from reaching
  # the TPM control protocol out-of-band.
  #
  # State lives under /var/lib/nixling/vms/<vm>/swtpm/.
  #
  # ---------------------------------------------------------------------------
  # P4 C3: nixling-<vm>-gpu.service — per-VM GPU + hypervisor sidecar.
  #
  # Runs the ENTIRE microvm-run (crosvm device gpu sidecar + cloud-hypervisor)
  # as the nixling-<vm>-gpu system user (not the host Wayland user). This satisfies C3:
  # `ps -ef | grep crosvm` shows nixling-<vm>-gpu, not the host Wayland user.
  #
  # Architecture note: microvm.nix's runner always spawns crosvm device gpu
  # inline (rm socket → fork crosvm → fork CH). Extracting ONLY crosvm-gpu
  # to a separate process requires patching the runner template, deferred:
  #
  # TODO P4 (deferred-to-followup): Pure crosvm-gpu extraction (CH as microvm
  # user, crosvm-gpu as nixling-<vm>-gpu) requires either:
  #   a) Override microvm.graphics.crosvmPackage with a setuid wrapper that
  #      delegates socket setup to nixling-<vm>-gpu.service, OR
  #   b) Patch microvm.nix's cloud-hypervisor.nix runner to skip inline spawn
  #      when a pre-existing socket is found.
  # ---------------------------------------------------------------------------
  systemd.services =
    (lib.mapAttrs' (name: _: lib.nameValuePair "nixling-${name}-swtpm" {
      description = "nixling swtpm 2.0 emulator for microVM ${name}";
      # Don't start unconditionally — start/restart on demand from the
      # nixling CLI's graphics-up flow. PartOf microvms.target so a
      # system-wide microvm restart cycles us too.
      partOf = [ "microvms.target" ];
      # v0.1.5: never restart on rebuild. swtpm changing under a
      # running VM means CH loses the TPM socket → guest TPM is wedged
      # → Entra/Intune device-bound creds become unreachable. The
      # consumer applies sidecar changes via `nixling restart <vm>`
      # (clean stop+start of the existing closure). Mirrors upstream
      # microvm.nix's X-RestartIfChanged=false on microvm@.
      #
      # v0.1.7 BUGFIX: pre-v0.1.7 this used
      # `unitConfig.X-RestartIfChanged = false`, which emits the
      # setting under the [Unit] section. NixOS's
      # switch-to-configuration logic only reads
      # `X-RestartIfChanged=` from the [Service] section, so the
      # `unitConfig` form was silently ignored and every
      # `nixos-rebuild switch` did cycle this sidecar. Use the
      # top-level `restartIfChanged = false` instead — NixOS
      # emits that under [Service] where the switch logic looks.
      restartIfChanged = false;
      serviceConfig = {
        Type = "exec";
        # H6: dedicated per-VM static user; DynamicUser removed so state is
        # stable and the migration ExecStartPre can reason about it.
        User = "nixling-${name}-swtpm";
        Group = "nixling-${name}-swtpm";
        DynamicUser = false;
        UMask = "0177";
        StateDirectory = "nixling/vms/${name}/swtpm";
        StateDirectoryMode = "0700";
        RuntimeDirectory = "swtpm/${name}";
        RuntimeDirectoryMode = "0711";
        # ExecStartPre migration is no longer needed: pre-Phase-2b state
        # migration (legacy /var/lib/private/swtpm/<vm>/ → new
        # /var/lib/private/nixling/vms/<vm>/swtpm/) is handled by the
        # Phase 9 migration script for consumers upgrading from
        # pre-public nixling.
        ExecStart = ''
          ${pkgs.swtpm}/bin/swtpm socket \
            --tpmstate dir=/var/lib/nixling/vms/${name}/swtpm \
            --ctrl type=unixio,path=/run/swtpm/${name}/sock,mode=0600 \
            --tpm2 \
            --flags startup-clear
        '';
        # H6: ACL-grant nixling-${name}-gpu rw on the socket after startup.
        # This allows cloud-hypervisor (running as nixling-${name}-gpu) to
        # connect to the TPM control socket.
        # -+ prefix: ignore failures (non-graphics VMs have no
        # nixling-${name}-gpu user) AND run as root (needed to setfacl on
        # the swtpm user's socket).
        ExecStartPost = "-+${pkgs.acl}/bin/setfacl -m u:nixling-${name}-gpu:rw /run/swtpm/${name}/sock";
        Restart = "on-failure";
        RestartSec = 2;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateDevices = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_UNIX" ];
        MemoryDenyWriteExecute = true;
        LockPersonality = true;
      };
    }) (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms))
    // (lib.mapAttrs' (name: _: lib.nameValuePair "nixling-${name}-gpu" {
      description = "nixling GPU+hypervisor sidecar for microVM ${name}";
      wants  = [ "network.target" "nixling-${name}-swtpm.service" "nixling-${name}-snd.service" ];
      after  = [ "network.target" "nixling-${name}-swtpm.service" "nixling-${name}-snd.service" ];
      wantedBy = [ ];
      # v0.1.5: never restart on rebuild. This sidecar IS the VM
      # runner (microvm-run / cloud-hypervisor); restarting it kills
      # the running VM, destroying any in-flight session state (login
      # session, Entra device-bound tokens in RAM, virtiofsd
      # connections, etc.). Consumer applies sidecar config changes
      # via `nixling restart <vm>` (clean stop+start of the existing
      # closure). Mirrors upstream microvm.nix's
      # X-RestartIfChanged=false on microvm@.
      #
      # v0.1.7 BUGFIX: pre-v0.1.7 this used
      # `unitConfig.X-RestartIfChanged = false`, which emits the
      # setting under the [Unit] section. NixOS's
      # switch-to-configuration logic only reads
      # `X-RestartIfChanged=` from the [Service] section, so the
      # `unitConfig` form was silently ignored and every
      # `nixos-rebuild switch` did cycle this sidecar. Use the
      # top-level `restartIfChanged = false` instead.
      restartIfChanged = false;
      serviceConfig = {
        Type = "exec";
        # C3: run as nixling-${name}-gpu — a dedicated system user,
        # NOT the host's Wayland user.
        User = "nixling-${name}-gpu";
        Group = "nixling-${name}-gpu";
        # security-r2-1: "audio" removed (see user declaration above).
        SupplementaryGroups = [ "kvm" ];
        WorkingDirectory = "/var/lib/nixling/vms/${name}";
        # C: private runtime dir for the sidecar. systemd creates
        # /run/nixling-gpu/${name}/ owned by nixling-${name}-gpu before start.
        RuntimeDirectory = "nixling-gpu/${name}";
        # C: rw ACL on the wayland socket only — NOT the parent /run/user/uid dir.
        # The socket is bind-mounted via BindPaths; sidecar never traverses /run/user/uid.
        ExecStartPre = [
          # v0.1.5: replicate microvm-set-booted_-start for graphics VMs.
          # Upstream microvm.nix's microvm-set-booted@<vm>.service only
          # runs when microvm@<vm>.service starts — which is bypassed for
          # graphics VMs (the GPU sidecar runs microvm-run directly).
          # Without `booted`, `nixling status`/`list` can't compute the
          # `current` vs `booted` mismatch that signals "config changed
          # while VM kept running; nixling switch <vm> needed". The `+`
          # prefix runs as root (the sidecar drops to nixling-<vm>-gpu
          # for the main ExecStart).
          ("+${pkgs.bash}/bin/bash -c '"
            + "rm -f /var/lib/nixling/vms/${name}/booted && "
            + "ln -s \"$(${pkgs.coreutils}/bin/readlink /var/lib/nixling/vms/${name}/current)\" "
            + "/var/lib/nixling/vms/${name}/booted'")
          ("+${pkgs.acl}/bin/setfacl -m u:nixling-${name}-gpu:rw /run/user/${waylandUid}/wayland-0")
        ];
        # Only wayland-0 is visible in the sidecar's mount namespace.
        BindPaths = [ "/run/user/${waylandUid}/wayland-0:/run/nixling-gpu/${name}/wayland-0" ];
        ExecStart = "/var/lib/nixling/vms/${name}/current/bin/microvm-run";
        # Graphics VMs bypass microvm@ and therefore need their own
        # post-start ACL refresh once cloud-hypervisor's API socket exists.
        ExecStartPost = lib.optionals (obsCfg.enable && obsCfg.ch.exporter.enable) [
          ("+${pkgs.bash}/bin/bash -c '"
            + "for i in $(${pkgs.coreutils}/bin/seq 1 120); do "
            + "  if [ -S /var/lib/nixling/vms/${name}/${name}.sock ]; then "
            + "    exec ${otelAclRefreshCommand}; "
            + "  fi; "
            + "  ${pkgs.coreutils}/bin/sleep 0.5; "
            + "done; "
            + "${otelAclRefreshCommand}'")
        ];
        Environment = [
          "WAYLAND_DISPLAY=wayland-0"
          "XDG_RUNTIME_DIR=/run/nixling-gpu/${name}"
        ];
        # Revoke wayland ACL on stop; remove booted symlink. Ignore failures.
        ExecStopPost = [
          ("-+${pkgs.acl}/bin/setfacl -x u:nixling-${name}-gpu /run/user/${waylandUid}/wayland-0")
          ("-+${pkgs.coreutils}/bin/rm -f /var/lib/nixling/vms/${name}/booted")
        ];
        Restart = "no";
        TimeoutStartSec = 120;
        TimeoutStopSec = 30;
        KillMode = "control-group";

        # ---- Sandboxing (Phase 2b H: GPU sidecar hardening) ----
        #
        # The GPU sidecar runs the entire microvm-run pipeline as a
        # dedicated system user. The directives below mirror the
        # audio sidecar's known-good template (components/audio/host.nix)
        # with the following deltas required by cloud-hypervisor +
        # crosvm device gpu:
        #
        # - DevicePolicy=closed + DeviceAllow for /dev/kvm and the
        #   render node. PrivateDevices is intentionally NOT set
        #   because we need the explicit device list to apply.
        # - RestrictAddressFamilies includes AF_VSOCK because CH
        #   uses vsock for sd_notify; AF_NETLINK for the tap helper.
        # - ReadWritePaths exposes the per-VM state dir (var.img,
        #   *.img, sockets) and the sidecar's RuntimeDirectory. The
        #   wayland socket is already bound via BindPaths.
        # - MemoryDenyWriteExecute is INTENTIONALLY OMITTED: crosvm
        #   needs PROT_WRITE+PROT_EXEC for its GPU command-buffer JIT.
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ReadWritePaths = [
          "/var/lib/nixling/vms/${name}"
          "/run/nixling-gpu/${name}"
        ];
        ProtectHome = true;
        PrivateTmp = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectProc = "invisible";
        RestrictAddressFamilies = [ "AF_UNIX" "AF_NETLINK" "AF_VSOCK" ];
        RestrictNamespaces = true;
        LockPersonality = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
        DevicePolicy = "closed";
        DeviceAllow = [
          "/dev/kvm rw"
          # Default render node on the host. Override per-VM via the
          # microvm.nix runner if the host has multiple GPUs.
          "/dev/dri/renderD128 rw"
          # v0.1.4 fix: cloud-hypervisor calls open("/dev/net/tun") +
          # ioctl(TUNSETIFF, …) to attach to its VM's tap. Pre-v0.1.4
          # this device wasn't in DeviceAllow, so DevicePolicy=closed
          # blocked the open() with EPERM and the VM crashed in early
          # boot with "Cannot create virtio-net device / Couldn't open
          # /dev/net/tun". The tap itself is created by upstream
          # microvm.nix's microvm-tap-interfaces@<vm>.service helper
          # (which runs as root, with CAP_NET_ADMIN); we only need
          # access to the cdev to attach.
          "/dev/net/tun rw"
        ];
        UMask = "0077";
      };
    }) (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms));
}
