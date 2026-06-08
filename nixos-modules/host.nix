{ inputs }:

{ config, pkgs, lib, ... }:

let
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) subnetIp mkMac;

  cfg = config.nixling;
  envMeta = cfg._envMeta;
  obsCfg = cfg.observability;
  obsVsockCid = 1000;
  obsOtlpPort = 14317;
  vmStateDir = name: "${cfg.store.stateDir}/${name}";
  obsVsockHostSocket = "${vmStateDir obsCfg.vmName}/vsock.sock";
  # microvm.nix's cloud-hypervisor runner removes `${vsockPath}_8888`
  # and binds the systemd-notify bridge there
  # (lib/runners/cloud-hypervisor.nix at
  # microvm-nix/microvm.nix@77024c22f4ddf509137fc732094888d1ffe631e2),
  # so the host-side AF_VSOCK backend uses a per-port suffix
  # convention: `<base>_<port>`. OTLP guest traffic on port 14317
  # therefore lands on `.../vsock.sock_14317`, not the base
  # `vsock.sock` mux/control socket.
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  obsOtlpVsockHostSocket = vsockSocketForPort obsVsockHostSocket obsOtlpPort;

  chVsockConnect = import ./nixling-ch-vsock-connect.nix { inherit pkgs; };

  # Phase 4 multi-arch gate: graphics + audio components both
  # transitively depend on x86_64-only packages (crosvm-patched,
  # spectrum-ch, vhost-device-sound — see their meta.platforms).
  # Headless workload VMs (no graphics, no audio) are arch-agnostic
  # and SHOULD evaluate on aarch64-linux per the published refactor
  # plan. The translation below throws an eval-time error with a
  # clear, attributable message the moment a VM declares
  # `graphics.enable = true` or `audio.enable = true` on any
  # non-x86_64-linux host platform, so the failure surfaces here
  # instead of producing a confusing downstream error inside the
  # component modules' let-bindings.
  hostSystem = pkgs.stdenv.hostPlatform.system;
  x86 = hostSystem == "x86_64-linux";
  checkVmPlatform = name: vm:
    if x86 || !(vm.graphics.enable || vm.audio.enable) then vm
    else throw ''
      nixling.vms.${name}: graphics/audio components are
      x86_64-linux only — refusing to evaluate on ${hostSystem}.
      The cloud-hypervisor + crosvm + vhost-device-sound pipeline
      (pkgs/spectrum-ch, pkgs/crosvm-patched, pkgs/vhost-device-sound)
      is gated to x86_64-linux via meta.platforms. Disable
      `graphics.enable` and `audio.enable` for VMs that must
      evaluate on ${hostSystem}, or evaluate this configuration
      against an x86_64-linux nixpkgs instance.
    '';

  # Fallback per-VM AF_VSOCK CID, derived from md5(vmName). Non-
  # observability VMs keep this for cloud-hypervisor's systemd-notify
  # path, which silences microvm.nix's eval-time
  #   "cloud-hypervisor supports systemd-notify via vsock, but
  #    microvm.vsock.cid must be set to enable this"
  # warning. Observability-enabled VMs instead pin the transport CID and
  # pass the host socket path via `microvm.cloud-hypervisor.extraArgs`,
  # which microvm.nix then folds into the final
  #   --vsock cid=...,socket=...
  # Cloud Hypervisor argument.
  #
  # CIDs 0/1/2 are reserved (hypervisor/local/host); we take 24 bits of
  # md5 (range 0..16_777_215) and offset by 4096 to stay clear of the
  # reserved range and any low CIDs other host tooling might grab.
  # Collision-resistant for ~1k VMs (birthday bound on 2^24 is ~4k).
  fallbackVsockCid = name:
    4096 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "md5" name));

  # Per-workload-VM derived values. Returns null when the VM has no
  # env (legacy mode — caller falls back to the VM's own
  # microvm.interfaces / systemd.network).
  vmDerive = name: vm:
    if vm.env == null || !(envMeta ? ${vm.env}) then null
    else
      let m = envMeta.${vm.env}; in {
        inherit (m) lanBridge uplinkBridge hostUplinkIp netLanIp mtu;
        envName = vm.env;
        mac = mkMac vm.env "lan" vm.index;
        ip = subnetIp m.lanSubnet vm.index;
        tap = "${vm.env}-l${toString vm.index}";
      };

  # Guest-side module auto-layered into any workload VM that names
  # an env. Provides:
  #   - the tap interface with the auto-derived MAC + tap name
  #   - a DHCP-only systemd.network block (dnsmasq on the net VM
  #     hands out the static reservation keyed off MAC). The IPv6
  #     LinkLocalAddressing/AcceptRA defaults from base.nix carry
  #     through; we add `UseDNS`/`UseRoutes` here for the v4 hints
  #     dnsmasq actually serves.
  # Replaces what the per-VM nix files used to declare by hand.
  envWorkloadGuestModule = derived: { lib, ... }: {
    microvm.interfaces = lib.mkForce [{
      type = "tap";
      id = derived.tap;
      mac = derived.mac;
    }];

    systemd.network.networks."10-eth-dhcp" = {
      matchConfig.Type = "ether";
      networkConfig = {
        DHCP = "ipv4";
        LinkLocalAddressing = "no";
        IPv6AcceptRA = false;
      };
      dhcpV4Config = {
        UseDNS = true;
        UseRoutes = true;
      };
    } // lib.optionalAttrs (derived.mtu != null) {
      linkConfig = {
        MTUBytes = toString derived.mtu;
      };
    };
  };

  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  # W3 (virt-3): systemd-supervised VMs only get a `microvm@<vm>`
  # template instance. Daemon-supervised VMs (`supervisor = "nixlingd"`)
  # are filtered out so the NixOS module never emits a parallel writer
  # for state the nixlingd daemon owns. The fail-fast stub unit
  # declared below makes the boundary explicit if an operator still
  # tries to start the unit via systemctl.
  systemdSupervisedVms =
    lib.filterAttrs (_: vm: vm.supervisor == "systemd") enabledVms;
  daemonSupervisedVmNames =
    lib.attrNames (lib.filterAttrs (_: vm: vm.supervisor == "nixlingd") enabledVms);

  usbipYubikeyVmEnabled =
    builtins.any (vm: vm.usbip.yubikey or false) (lib.attrValues enabledVms);

  workloadObsVmNames =
    lib.attrNames (lib.filterAttrs (_: vm: vm.observability.enable) enabledVms);

  # `transport-vsock` lands before `observability-vm.nix`, so don't
  # auto-start relay instances until the auto-declared obs VM actually
  # exists in `cfg.vms`.
  obsVmEnabled =
    obsCfg.enable
    && cfg.vms ? ${obsCfg.vmName}
    && cfg.vms.${obsCfg.vmName}.enable;

  relayVmNames =
    if obsVmEnabled
    then lib.filter (name: name != obsCfg.vmName) workloadObsVmNames
    else [ ];

  # Graphics VMs run via nixling-<vm>-gpu.service (the GPU sidecar IS
  # the CH runner — they bypass microvm@). Headless VMs use microvm@.
  # Both runners need to start the per-VM relay so the host-side vsock
  # bridge actually connects. Templated systemd units can't have
  # per-instance BindsTo/Wants, so we wire per-VM `wants` on each
  # graphics gpu sidecar separately below. (panel-w3r3)
  graphicsRelayVmNames =
    lib.filter (name: (cfg.vms.${name}.graphics.enable or false)) relayVmNames;

  relayEligibilityScript = pkgs.writeShellScript "nixling-otel-relay-eligible" ''
    case " ${lib.concatStringsSep " " relayVmNames} " in
      *" $1 "*) exit 0 ;;
      *) exit 1 ;;
    esac
  '';

  observabilityVsock = name: vm:
    if obsCfg.enable && name == obsCfg.vmName then {
      cid = obsVsockCid;
      socket = obsVsockHostSocket;
    } else if vm.observability.enable then {
      cid = config.nixling.manifest.${name}.observability.vsockCid;
      socket = config.nixling.manifest.${name}.observability.vsockHostSocket;
    } else null;

  vsockStateDirVmNames =
    lib.unique (workloadObsVmNames ++ lib.optional obsCfg.enable obsCfg.vmName);

in

{
  imports = [
    inputs.microvm.nixosModules.host
    ./host-users.nix
    ./host-polkit.nix
    ./host-activation.nix
    ./host-keys.nix
    ./host-ssh-host-keys.nix
    ./observability-host-secrets.nix
    ./host-daemon.nix
  ];

  # Store per-VM state under /var/lib/nixling/vms/<vm>/ instead of
  # microvm.nix's default /var/lib/microvms/<vm>/. Keeps every
  # nixling-managed file under one tree:
  #
  #   /var/lib/nixling/
  #     vms/<vm>/                            workload + sys VMs
  #       state/audio-state.json
  #       swtpm/                             per-VM TPM state
  #       store/, store-meta/                per-VM nix store
  #       host-keys/                         per-VM nixling-managed
  #                                          host pubkey + user keys
  #     keys/                                nixling-managed SSH keys
  #
  # Pre-Phase-2b consumers upgrading from an earlier nixling layout
  # (where workload state lived under /var/lib/microvms/<vm>/ or
  # /var/lib/nixling/<vm>/) should use the Phase 9 migration script
  # to move their state into vms/<vm>/. New installs land on this
  # layout directly.
  #
  # Note: with microvm.nix's current single-global stateDir, the auto-
  # declared system VMs (`sys-<env>-net`) also land under
  # /var/lib/nixling/vms/sys-<env>-net/. The split into a separate
  # `sys/<env>-net/` tree requires either a microvm.nix patch
  # exposing a per-VM stateDir override, or filesystem-level
  # bind-mounts. Tracked for a future phase.
  microvm.stateDir = cfg.store.stateDir;

  # P6 (ph6-remove-systemd-emission): host-wrapper.nix used to emit
  # the `nixling@<vm>.service` template plus the per-instance
  # `systemd.targets.multi-user.wants` symlinks for autostart=true
  # VMs. Both are deleted: the daemon (`nixlingd`) owns VM lifecycle
  # end-to-end via broker `SpawnRunner{role: CloudHypervisor}` and
  # the supervisor's `pidfd` watchdog. Autostart is now expressed as
  # `nixling.vms.<vm>.autostart = true` in the bundle and consumed
  # by `nixlingd::autostart` at daemon startup, not by systemd
  # target.wants.
  #
  # The `microvm.autostart` / `systemd.targets.microvms.wants`
  # forces below remain so that any residual `microvm@<vm>.service`
  # unit files generated by the upstream microvm.nix host module
  # (still imported above for option-schema reasons) do NOT get
  # pulled into `multi-user.target` at boot.
  microvm.autostart = [ ];
  systemd.targets.microvms.wants = lib.mkForce [ ];

  # Translate each enabled nixling.vms.<name> into microvm.vms.<name>.
  # `specialArgs` is fixed here so VMs can pull from the same flake
  # graph without each VM-file having to plumb it manually. Every VM
  # gets `./base.nix` layered in for the common guest baseline
  # (networkd, sshd, resolved, locale, stateVersion).
  #
  # Note: `autostart` is intentionally NOT propagated from
  # nixling.vms.<name>.autostart into microvm.vms.<name>.autostart.
  # Upstream microvm.nix would accumulate autostart entries from each
  # microvm.vms.<name> with autostart=true into `microvm.autostart`,
  # which would re-introduce `WantedBy=multi-user.target` onto
  # `microvm@<vm>.service`. Nixling instead pins
  # `microvm.autostart = []` (in ./host-wrapper.nix) and sets WantedBy
  # on its own `nixling@<vm>.service` wrapper per-instance there.
  microvm.vms = lib.mapAttrs
    (name: vm:
      let
        # Phase 4 multi-arch: force the platform gate before the rest
        # of the translation observes `vm`. `checkVmPlatform` either
        # returns `vm` unchanged (x86_64-linux, or no x86_64-only
        # components requested) or throws a clear, attributable error
        # naming the VM. Forced via `let _ = …` so non-strict consumers
        # still trip it.
        vm' = checkVmPlatform name vm;
        derived = vmDerive name vm';
        chVsock = observabilityVsock name vm';
      in {
        specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;
        config = lib.mkMerge [
          {
            imports = [
              ./base.nix
              ./guest-sshd-host-keys.nix
            ]
              ++ lib.optional vm'.graphics.enable ./components/graphics.nix
              ++ lib.optional vm'.tpm.enable ./components/tpm.nix
              ++ lib.optional vm'.usbip.yubikey ./components/usbip.nix
              ++ lib.optional vm'.audio.enable ./components/audio/guest.nix
              ++ lib.optional vm'.audit.enable ./components/audit.nix
              ++ lib.optional vm'.graphics.enable ./components/video/guest.nix
              ++ lib.optional vm'.observability.enable ./components/observability/guest.nix
              # Note: Entra ID / Himmelblau is NOT a nixling component.
              # Consumers who need it import it per-VM via:
              #   nixling.vms.<vm>.config.imports = [
              #     inputs.nixos-entra-id.nixosModules.default
              #   ];
              # See `vicondoa/nixos-entra-id` for the sibling flake.
              ++ lib.optional vm'.homeManager.enable ./components/home-manager.nix
              ++ lib.optional (derived != null) (envWorkloadGuestModule derived)
              ++ [ vm'.config ];
          }
          (lib.mkIf (chVsock == null) {
            # Non-observability VMs keep the fallback per-VM vsock CID
            # for cloud-hypervisor's systemd-notify path.
            microvm.vsock.cid = lib.mkDefault (fallbackVsockCid name);
          })
          (lib.mkIf (chVsock != null) {
            microvm.hypervisor = lib.mkDefault "cloud-hypervisor";
            microvm.vsock.cid = lib.mkForce chVsock.cid;
            microvm.cloud-hypervisor.extraArgs = lib.mkAfter [
              "--vsock"
              "socket=${chVsock.socket}"
            ];
          })
          # Propagate host-side per-component config into the guest's
          # matching option set. Each branch is gated on the matching
          # toggle so the options-don't-exist error doesn't fire when
          # the component module isn't imported. The mkIf must wrap the
          # WHOLE module attrset (not just the inner value) for option
          # resolution to see the definition as absent rather than as a
          # mkIf-tagged-condition-false definition of a missing option.
          (lib.mkIf vm'.homeManager.enable {
            nixling.homeManager.users = vm'.homeManager.users;
          })
          # P5 W3: propagate the host-side cross-domain trust flag to
          # the guest so graphics.nix can gate the crosvm GPU sidecar's
          # cross-domain context type on it.
          (lib.mkIf vm'.graphics.enable {
            nixling.graphics.crossDomainTrusted = vm'.graphics.crossDomainTrusted;
          })
          # Propagate the audio-user list to the guest. Default falls
          # back to `[ ssh.user ]` if the per-VM file didn't override
          # `audio.users` explicitly. The guest module adds each user
          # to the `audio` group so they can talk to the virtio-snd
          # device + PipeWire from non-logind-active sessions.
          (lib.mkIf vm'.audio.enable {
            nixling.audio.users =
              if vm'.audio.users != [ ]
              then vm'.audio.users
              else lib.optional (vm'.ssh.user != null) vm'.ssh.user;
          })
          (lib.mkIf vm'.audit.enable {
            nixling.audit.enable = true;
            nixling.audit.rules = vm'.audit.rules;
          })
          (lib.mkIf vm'.observability.enable {
            nixling.observability.scrapeJournal = vm'.observability.scrapeJournal;
            nixling.observability.scrapeNodeMetrics = vm'.observability.scrapeNodeMetrics;
            nixling.observability.identity.vmName = name;
            nixling.observability.identity.envName = if vm'.env != null then vm'.env else "none";
          })
          # Propagate the SSH user to the guest so
          # nixling-load-host-keys.service knows whose
          # authorized_keys to populate (Phase 2b nixling-managed keys).
          { nixling.sshUser = vm'.ssh.user;
            nixling.sudo = vm'.sudo;
          }
        ];
      })
    systemdSupervisedVms;

  # W3 (virt-3): emit fail-fast stub `microvm@<vm>.service` units for
  # every daemon-supervised VM. The systemd-side stub returns
  # `single-writer-conflict` (exit 78) so an operator running
  # `systemctl start microvm@<vm>` against a daemon-owned VM gets a
  # clear pointer to `nixling host prepare` + the nixlingd daemon
  # path. `RestartPolicy = "no"` and the `ConditionPathExists`
  # negative match on the daemon-owned marker keep the stub from
  # firing once the daemon has taken ownership.
  systemd.services = lib.mkMerge [
    (lib.genAttrs
      (map (name: "microvm@${name}") daemonSupervisedVmNames)
      (unitName:
        let
          name = lib.removePrefix "microvm@" unitName;
        in {
          description = "nixling W3 single-writer stub for ${name} (daemon-supervised)";
          restartIfChanged = false;
          unitConfig = {
            ConditionPathExists = "!/run/nixling/state/${name}/owned-by-daemon";
            X-RestartIfChanged = false;
          };
          serviceConfig = {
            Type = lib.mkForce "oneshot";
            RemainAfterExit = lib.mkForce false;
            StandardOutput = "journal";
            StandardError = "journal";
            Restart = lib.mkForce "no";
            SuccessExitStatus = "";
          };
          # Surface a clear pointer at the daemon path. Exit 78 is the
          # documented `single-writer-conflict` / config-mismatch code.
          script = ''
            echo "nixling: microvm@${name}.service refused: VM '${name}' is supervised by the nixlingd daemon (single-writer-conflict)." >&2
            echo "Remediation: use 'nixling host prepare' / 'nixling up ${name}' instead of 'systemctl start microvm@${name}'." >&2
            exit 78
          '';
        }))
    # P6 ph6-remove-systemd-emission + P3 ph3-p3-otelbridge-readiness:
    # the per-VM `nixling-otel-relay@<vm>.service` template + drop-ins
    # were deleted. The observability vsock relay is now broker-spawned
    # via `SpawnRunner{role: VsockRelay}` (P1 ph1-vsock-relay role) with
    # readiness gated by P3 ph3-p3-otelbridge-readiness. The
    # `systemd.services."nixling-otel-relay@"` template + the per-VM
    # `nixling-otel-relay@<vm>` drop-ins that were here have been
    # removed. See docs/reference/otel-host-bridge-readiness.md +
    # docs/adr/0015-daemon-only-clean-break.md.
    (lib.mkIf false {})
  ];

  # H5: restrict /dev/kvm to kvm group members only (was world-rwx 0666).
  # Only nixling-<vm>-gpu system users and the microvm service user are in
  # the kvm group after P4; the host's Wayland user is no longer a member.
  # The /dev/kvm rule is unconditional; Yubico-specific rules are gated on
  # `nixling.site.yubikey.enable` (W3b H4 — the option was previously
  # declared but unused).
  services.udev.extraRules = ''
    # H5 — lock KVM device to kvm group, no longer world-accessible
    KERNEL=="kvm", GROUP="kvm", MODE="0660"
  '' + lib.optionalString config.nixling.site.yubikey.enable ''
    # Yubico YubiKey — hidraw (FIDO/U2F)
    SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1050", GROUP="kvm", MODE="0660", TAG+="uaccess"
    # Yubico YubiKey — raw USB device
    SUBSYSTEM=="usb", ATTRS{idVendor}=="1050", GROUP="kvm", MODE="0660", TAG+="uaccess"
  '';

  # USBIP host kernel module — `nixling-sys-<env>-usbipd-proxy.service`
  # instances live in network.nix (one per env, each bound to its
  # uplink bridge IP). Load it only when host-side YubiKey support is
  # enabled AND at least one enabled VM opts into `usbip.yubikey`,
  # matching the per-VM USBIP materialization gate.
  boot.kernelModules = lib.mkIf
    (cfg.site.yubikey.enable && usbipYubikeyVmEnabled)
    [ "usbip-host" ];

  # Host-side debug binaries + acl (setfacl used in activation scripts and
  # nixling-<vm>-{gpu,snd,swtpm} ExecStartPre/Post stanzas):
  environment.systemPackages = [
    pkgs.linuxPackages.usbip
    pkgs.swtpm
    pkgs.tpm2-tools
    pkgs.acl
  ];

  # P2r4/P4: pre-create /run/nixling and lock file at boot.
  # Lock file is group=nixling-launcher so members of that group can
  # open it with `exec 9>` without write access to root:root 0755
  # /run/nixling.
  #
  # `/run/nixling/vms/` is the parent for per-VM RuntimeDirectory= entries
  # (e.g. /run/nixling/vms/<vm>/snd.sock from the audio sidecar). Pre-creating
  # it root:root 0755 means systemd's RuntimeDirectory creation only owns
  # the leaf <vm>/ dir, keeping the shared parent under root ownership.
  # `/run/nixling/alloy/` is a private subtree for observability sockets so
  # Alloy no longer needs write access to the shared launcher/audio lock root.
  # P4 C3: also pre-create the GPU sidecar's runtime root.
  # P0 (ph0-runtime-dir-canonicalize): when daemonExperimental is
  # enabled, /run/nixling is owned exclusively by host-daemon.nix
  # (nixlingd:nixling-launchers 0750). This module emits its OWN
  # /run/nixling rule only in the pre-daemon path. The duplicate
  # `if daemonExperimental … 0755 root root` form is REMOVED.
  systemd.tmpfiles.rules = lib.optionals (! cfg.daemonExperimental.enable) [
    "d /run/nixling             0775 root nixling-launcher -"
  ] ++ [
    "d /run/nixling/vms         0755 root root -"
    "f /run/nixling/usbipd.lock 0660 root nixling-launcher -"
    "d /run/nixling-gpu         0755 root root -"
    # P7r2 security-r7-2: lock file for nixling-known-hosts-refresh@.service
    # so concurrent refresh runs (one per VM at boot) serialize against the
    # same file the CLI do_trust path uses. Mode 0660 root:nixling-launcher
    # so launcher-group members can also flock it from `nixling trust`.
    "f ${cfg.site.stateDir}/known_hosts.nixling.lock 0660 root nixling-launcher -"
    # Phase 2b reserve: keys directory for nixling-managed SSH keys.
    # Created root:root 0700 — Phase 2b's generator activation script
    # (deferred) will populate it.
    "d ${cfg.site.keysDir} 0710 root nixling-launcher -"
    "D ${cfg.site.tmpDir} 0755 root root -"
  ]
  # /run/nixling/alloy is created at service-start time by
  # alloy.service's `RuntimeDirectory=nixling/alloy` directive, not
  # via tmpfiles — the alloy account is a systemd DynamicUser whose
  # UID/GID is only allocated at start, and tmpfiles cannot chown
  # to that user at activation time. nixling-otel-host-bridge's
  # ExecStartPre setfacl runs AFTER alloy.service (After= + bindsTo)
  # so the directory is guaranteed to exist by then.
  # Observability VMs need their per-VM state dir present before
  # cloud-hypervisor binds `.../vsock.sock` there.
  ++ map
    (name: "d ${vmStateDir name} 2770 microvm kvm -")
    vsockStateDirVmNames;
}
