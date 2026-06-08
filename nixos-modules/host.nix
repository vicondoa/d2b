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
        inherit (m) lanBridge uplinkBridge hostUplinkIp netLanIp;
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
    };
  };

  workloadObsVmNames =
    lib.attrNames
      (lib.filterAttrs (_: vm: vm.enable && vm.observability.enable) cfg.vms);

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
    ./host-wrapper.nix
    ./host-polkit.nix
    ./host-sidecars.nix
    ./host-activation.nix
    ./host-keys.nix
    ./host-ssh-host-keys.nix
    ./host-known-hosts.nix
    ./host-audit.nix
    ./observability-host-secrets.nix
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
  microvm.stateDir = "/var/lib/nixling/vms";

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
          (lib.mkIf vm'.observability.enable {
            nixling.observability.scrapeJournal = vm'.observability.scrapeJournal;
            nixling.observability.scrapeNodeMetrics = vm'.observability.scrapeNodeMetrics;
            nixling.observability.identity.vmName = name;
            nixling.observability.identity.envName = if vm'.env != null then vm'.env else "none";
          })
          # Propagate the SSH user to the guest so
          # nixling-load-host-keys.service knows whose
          # authorized_keys to populate (Phase 2b nixling-managed keys).
          { nixling.sshUser = vm'.ssh.user; }
        ];
      })
    (lib.filterAttrs (_: vm: vm.enable) cfg.vms);

  # Per-VM host relay between the workload VM's Cloud Hypervisor vsock
  # backend socket and the observability stack VM's backend socket.
  #
  # The relay is attached to `microvm@%i.service` instead of being put
  # directly in `multi-user.target.wants`: the latter would start the
  # relay at boot, and the relay's `BindsTo=microvm@%i.service` would in
  # turn boot every monitored VM even when `autostart = false`.
  systemd.services = lib.mkIf (obsCfg.enable && relayVmNames != [ ]) (lib.mkMerge [
    {
      "nixling-otel-relay@" = {
        description = "Host byte-relay between %i vsock backend and the obs-VM vsock backend.";
        # BindsTo intentionally NOT applied to the runner. Templated units
        # can't conditionally bind to microvm@<vm>.service vs
        # nixling-<vm>-gpu.service per VM. Instead the runtime gates
        # below (ExecCondition + ExecStartPre socket check + Restart=on-failure)
        # keep the relay in sync with whichever runner is active.
        # (panel-w3r3 software-1 / nixos-1 / networking-1 / observability-1)
        after = [ "microvm@${obsCfg.vmName}.service" ];
        bindsTo = [ "microvm@${obsCfg.vmName}.service" ];
        restartIfChanged = false;
        startLimitIntervalSec = 300;
        startLimitBurst = 20;
        serviceConfig = {
          Type = "exec";
          ExecCondition = "${relayEligibilityScript} %i";
          # NOTE: do NOT `test -S` the per-port vsock socket paths
          # (`vsock.sock_${obsOtlpPort}`). For guest→host vsock
          # (which is the direction the workload uses), CH doesn't
          # create the host UDS — the host has to create it as a
          # LISTENer, and CH opens a fresh connection to it for
          # each guest VSOCK-CONNECT. Check the base CH vsock UDS
          # (which is always present once microvm@<vm> is active)
          # so we still gate on the VM being up.
          ExecStartPre = [
            "${pkgs.coreutils}/bin/test -S ${vmStateDir "%i"}/vsock.sock"
            "${pkgs.coreutils}/bin/test -S ${vmStateDir obsCfg.vmName}/vsock.sock"
            # Clean stale UDS from a prior crashed relay instance.
            # socat will fail to bind UNIX-LISTEN otherwise.
            "+${pkgs.coreutils}/bin/rm -f ${vsockSocketForPort "${vmStateDir "%i"}/vsock.sock" obsOtlpPort}"
          ];
          # OTLP push path: workload VM's guest socat does
          # VSOCK-CONNECT:2:14317. CH proxies that to a UNIX
          # connect against ${vmStateDir "%i"}/vsock.sock_14317
          # on the host — which means the host needs a LISTENer
          # at that path. THIS relay is that listener. On accept,
          # the relay forks and immediately opens the obs stack VM's
          # vsock via the CH textual protocol (EXEC helper). The
          # stack VM has VSOCK-LISTEN:14317 internally; CH does NOT
          # auto-create per-port host UDS files for host-to-guest
          # direction, so the textual protocol on the base UDS is
          # the only way to reach the stack from the host.
          ExecStart = ''
            ${obsCfg.transport.relayPackage}/bin/socat -d -d \
              UNIX-LISTEN:${vsockSocketForPort "${vmStateDir "%i"}/vsock.sock" obsOtlpPort},fork,reuseaddr,mode=0660 \
              EXEC:"${chVsockConnect}/bin/nixling-ch-vsock-connect ${vmStateDir obsCfg.vmName}/vsock.sock ${toString obsOtlpPort}"
          '';
          Restart = "on-failure";
          RestartSec = "3s";
          DynamicUser = true;
          SupplementaryGroups = [ "kvm" ];
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          PrivateDevices = true;
          RestrictAddressFamilies = [ "AF_UNIX" ];
          SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
          CapabilityBoundingSet = "";
          AmbientCapabilities = "";
          ReadWritePaths = [
            # ProtectSystem=strict makes / read-only except for these.
            # CH lazily materialises vsock.sock_<port> in the per-VM
            # state dir on the FIRST UNIX-CONNECT — we need write to
            # the parent directory so the new inode can be created
            # in response to our connect attempt. Pre-v0.2.0 this
            # block listed the per-port file paths directly, but
            # systemd's mount-namespace setup tries to bind-mount each
            # listed path before the unit's main process runs, and
            # bind-mounting a non-existent file aborts the namespace
            # setup with ENOENT (which we then saw as the relay
            # silently skipping on ExecCondition failure).
            (vmStateDir "%i")
            (vmStateDir obsCfg.vmName)
          ];
        };
      };

      "microvm@" = {
        wants = [ "nixling-otel-relay@%i.service" ];
      };
    }
    # Per-graphics-VM wiring: each graphics VM's gpu sidecar (which IS
    # its CH runner) pulls in its own relay. (panel-w3r3)
    (lib.listToAttrs (lib.map
      (vmName: lib.nameValuePair "nixling-${vmName}-gpu" {
        wants = [ "nixling-otel-relay@${vmName}.service" ];
      })
      graphicsRelayVmNames))
  ]);

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
  # uplink bridge IP). Loading is gated on `nixling.site.yubikey.enable`
  # because the only consumer in the v0.1.0 surface is the Yubikey
  # passthrough path; hosts that disable yubikey support don't need
  # the kernel module either (it would only sit idle).
  boot.kernelModules =
    lib.optional config.nixling.site.yubikey.enable "usbip-host";

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
  systemd.tmpfiles.rules = [
    "d /run/nixling             0755 root root -"
    "d /run/nixling/vms         0755 root root -"
    "f /run/nixling/usbipd.lock 0660 root nixling-launcher -"
    "d /run/nixling-gpu         0755 root root -"
    # P7r2 security-r7-2: lock file for nixling-known-hosts-refresh@.service
    # so concurrent refresh runs (one per VM at boot) serialize against the
    # same file the CLI do_trust path uses. Mode 0660 root:nixling-launcher
    # so launcher-group members can also flock it from `nixling trust`.
    "f /var/lib/nixling/known_hosts.nixling.lock 0660 root nixling-launcher -"
    # Phase 2b reserve: keys directory for nixling-managed SSH keys.
    # Created root:root 0700 — Phase 2b's generator activation script
    # (deferred) will populate it.
    "d /var/lib/nixling/keys    0700 root root -"
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
