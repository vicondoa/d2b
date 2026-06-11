{ inputs }:

{ config, pkgs, lib, ... }:

let
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) subnetIp mkMac;
  # Per-VM evaluator entry point. composeVm is the nixling-owned
  # replacement for microvm.nix's per-VM
  # lib.evalModules invocation; see vm-evaluator.nix +
  # vm-options.nix. host.nix's `nixling._computed` mapping below
  # calls composeVm for every enabled VM and stores the result
  # at `config.nixling._computed.<name>.config.*`, where the
  # lib.nix helpers (vmRunner / vmToplevel / vmDeclaredRunner)
  # read it.
  vmSubmodule = (import ./vm-submodule.nix { inherit inputs; })
    { inherit config lib pkgs; };
  composeVm = vmSubmodule._composeVm;

  cfg = config.nixling;
  envMeta = cfg._envMeta;
  obsCfg = cfg.observability;
  vmStateDir = name: "${cfg.store.stateDir}/${name}";

  # graphics + audio components both
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
  # an env. Provides
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
  # `nixling.vms.<vm>.supervisor` was removed per ADR 0015.
  # Every enabled VM is now daemon-supervised; the systemd-template
  # path is retired. The empty `systemdSupervisedVms` set keeps
  # legacy iteration sites from emitting per-VM microvm@<vm>
  # template instances; the v1.1- phase deletes those sites
  # outright when the template definitions themselves go.
  systemdSupervisedVms = { };
  daemonSupervisedVmNames = lib.attrNames enabledVms;

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
  # graphics gpu sidecar separately below.
  graphicsRelayVmNames =
    lib.filter (name: (cfg.vms.${name}.graphics.enable or false)) relayVmNames;

  relayEligibilityScript = pkgs.writeShellScript "nixling-otel-relay-eligible" ''
    case " ${lib.concatStringsSep " " relayVmNames} " in
      *" $1 "*) exit 0 ;;
      *) exit 1 ;;
    esac
  '';

  baseVsock = name: {
    cid = config.nixling.manifest.${name}.observability.vsockCid;
    socket = config.nixling.manifest.${name}.observability.vsockHostSocket;
  };

  vsockStateDirVmNames =
    lib.unique (workloadObsVmNames ++ lib.optional obsCfg.enable obsCfg.vmName);

in

{
  imports = [
    # inputs.microvm.nixosModules.host import REMOVED.
    # The nixling-owned per-VM evaluator (vm-evaluator.nix +
    # vm-options.nix) replaces microvm.nix's host module. The
    # per-VM `microvm.*` option namespace lives inside the per-VM
    # NixOS evaluation only (via vm-options.nix); no host-level
    # `microvm.*` option set is exposed. The systemd
    # `microvm@<vm>.service` / `microvm-virtiofsd@<vm>.service`
    # templates microvm.nix would have emitted are not declared;
    # the broker SpawnRunner pipeline owns every spawn directly.
    ./host-users.nix
    ./host-polkit.nix
    ./host-activation.nix
    ./host-keys.nix
    ./host-ssh-host-keys.nix
    ./observability-host-secrets.nix
    ./host-daemon.nix
  ];

  # Per-VM state directory layout. /var/lib/nixling/vms/<vm>/ carries
  # every nixling-managed file for the VM (no more
  # microvm.nix-driven /var/lib/microvms/<vm>/ path; nixling owns
  # the substrate end-to-end). The `microvm.stateDir`,
  # `microvm.autostart`, and `systemd.targets.microvms.wants`
  # assignments that lived here under v1.0 are deleted with the
  # upstream microvm.nix host module import.

  # Per-VM NixOS evaluation lives on the nixling-owned
  # `nixling._computed.<name>` attribute (see vm-evaluator.nix).
  # Populated via the composeVm closure imported at the top of this
  # module. Storage location is `nixling._computed` (sibling to
  # `nixling.vms`) rather than `nixling.vms.<name>.computed` to
  # avoid module-system infinite recursion (a mapAttrs over
  # cfg.vms cannot write back to the same nixling.vms attribute
  # path it reads from).
  nixling._computed = lib.mapAttrs
    (name: vm:
      let
        vm' = checkVmPlatform name vm;
        derived = vmDerive name vm';
        chVsock = baseVsock name;
        obsCfg = cfg.observability;
        isObsVm = obsCfg.enable && name == obsCfg.vmName;
        obsSecretsShare = lib.optional isObsVm {
          source = "${cfg.site.stateDir}/observability";
          mountPoint = "/run/nixling-obs-secrets";
          tag = "nl-obs-sec";
          proto = "virtiofs";
        };
        guestControlShare = lib.optional vm'.guest.control.enable {
          source = "${cfg.site.stateDir}/guest-control-${name}";
          mountPoint = "/run/nixling-guest-control-host";
          tag = "nl-gctl";
          proto = "virtiofs";
          readOnly = true;
        };
        composedModules = [
          # Framework guest baseline + nixling-managed sshd host keys.
          ./base.nix
          ./guest-sshd-host-keys.nix
        ]
          ++ lib.optional vm'.graphics.enable ./components/graphics.nix
          ++ lib.optional vm'.tpm.enable ./components/tpm.nix
          ++ lib.optional vm'.usbip.yubikey ./components/usbip.nix
          ++ lib.optional vm'.audio.enable ./components/audio/guest.nix
          ++ lib.optional vm'.audit.enable ./components/audit.nix
          ++ lib.optional (vm'.graphics.enable && vm'.graphics.videoSidecar) ./components/video/guest.nix
          ++ lib.optional vm'.observability.enable ./components/observability/guest.nix
          ++ lib.optional vm'.homeManager.enable ./components/home-manager.nix
          ++ lib.optional (derived != null) (envWorkloadGuestModule derived)
          ++ [ vm'.config ]
          ++ lib.optional (vm'.guestConfigFile != null) vm'.guestConfigFile
          # Seed the guest-editable config INTO the VM so an operator can
          # see + edit it from inside the guest, then `nixling config
          # sync` it back. The read-only baseline always reflects the
          # currently-approved guestConfigFile; a writable working copy
          # is seeded once (tmpfiles `C` = copy-if-absent) for the SSH
          # user to edit. No new host surface — it rides the normal
          # read-only closure (no virtiofs share).
          ++ lib.optional (vm'.guestConfigFile != null) (
            { lib, ... }: {
              environment.etc."nixling/guest-config.nix".source = vm'.guestConfigFile;
              systemd.tmpfiles.rules = lib.optionals (vm'.ssh.user != null) [
                "d /var/lib/nixling-guest 0750 ${vm'.ssh.user} users -"
                "C /var/lib/nixling-guest/guest-config.nix 0640 ${vm'.ssh.user} users - /etc/nixling/guest-config.nix"
              ];
            }
          )
          ++ [
            {
              microvm.vsock.cid = lib.mkForce chVsock.cid;
              microvm.vsock.socket = lib.mkForce chVsock.socket;
            }
          ]
          ++ lib.optional vm'.homeManager.enable {
            nixling.homeManager.users = vm'.homeManager.users;
          }
          ++ lib.optional vm'.graphics.enable {
            nixling.graphics.crossDomainTrusted = vm'.graphics.crossDomainTrusted;
            nixling.graphics.xwayland.enable = vm'.graphics.xwayland.enable;
            nixling.graphics.virglVideo = vm'.graphics.virglVideo;
          }
          ++ lib.optional vm'.audio.enable {
            nixling.audio.users =
              if vm'.audio.users != [ ]
              then vm'.audio.users
              else lib.optional (vm'.ssh.user != null) vm'.ssh.user;
          }
          ++ lib.optional vm'.audit.enable {
            nixling.audit.enable = true;
            nixling.audit.rules = vm'.audit.rules;
          }
          ++ lib.optional vm'.observability.enable {
            nixling.observability.scrapeJournal = vm'.observability.scrapeJournal;
            nixling.observability.scrapeNodeMetrics = vm'.observability.scrapeNodeMetrics;
            nixling.observability.identity.vmName = name;
            nixling.observability.identity.envName = if vm'.env != null then vm'.env else "none";
          }
          ++ [
            {
              nixling.sshUser = vm'.ssh.user;
              nixling.sudo = vm'.sudo;
              nixling.guestControl.enable = vm'.guest.control.enable;
              nixling.guestControl.exec = {
                enable = lib.mkForce vm'.guest.exec.enable;
                allowRoot = lib.mkForce vm'.guest.exec.allowRoot;
                users = lib.mkForce vm'.guest.exec.users;
              };
            }
            # Per-VM framework-managed shares moved from store.nix to
            # break the module-system infinite
            # recursion store.nix would cause when mapping over
            # cfg.vms to write back to nixling.vms).
            {
              # writableStoreOverlay is backed by a broker-provisioned
              # disk image at `${cfg.store.stateDir}/<vm>/store-overlay.img`.
              # Per-VM config can set `writableStoreOverlay = "/nix/.rw-store"`
              # again. The broker creates the disk image before SpawnRunner
              # via the `DiskInit` plan-op emitted by processes-json.nix.
              microvm.shares = lib.mkForce ([
                {
                  source = "/nix/store";
                  mountPoint = "/nix/.ro-store";
                  tag = "ro-store";
                  proto = "virtiofs";
                }
                {
                  source = "${cfg.store.stateDir}/${name}/store-meta";
                  mountPoint = "/run/nixling-store-meta";
                  tag = "nl-meta";
                  proto = "virtiofs";
                }
                {
                  source = "${cfg.site.stateDir}/vms/${name}/host-keys";
                  mountPoint = "/run/nixling-host-keys";
                  tag = "nl-hkeys";
                  proto = "virtiofs";
                }
                {
                  source = "${cfg.site.stateDir}/vms/${name}/sshd-host-keys";
                  mountPoint = "/run/nixling-sshd-host-keys";
                  tag = "nl-ssh-host";
                  proto = "virtiofs";
                }
              ] ++ obsSecretsShare ++ guestControlShare);
            }
          ];
      in (composeVm name composedModules) // {
        # Namespace-containment policy lint for the guest-editable
        # `guestConfigFile`: evaluated over the real nixpkgs NixOS module
        # set (see lib.nix) with the same pkgs/specialArgs the per-VM
        # evaluator uses, so a guest config that reads standard options
        # resolves instead of false-positiving. Forbidden namespaces are
        # detected by definition-existence (imports / generated modules /
        # `_file` spoofing all caught). This is NOT an eval-time security
        # sandbox — see lib.nix + docs/adr/0024 for the trust model.
        guestForbidden =
          if vm'.guestConfigFile == null then [ ]
          else nl.guestConfigForbiddenNamespaces
            {
              inherit pkgs;
              specialArgs = { inherit inputs; name = name; } // cfg.site.extraSpecialArgs;
            }
            vm'.guestConfigFile;
      })
    enabledVms;

  # Fail-fast stub `microvm@<vm>.service` units are no longer needed —
  # `microvm@<vm>.service` doesn't exist anymore
  # (the upstream microvm.nix host module that declared it is no
  # longer imported). Operators interacting via systemctl get
  # systemd's standard "unknown unit" message; the daemon-owned
  # broker SpawnRunner pipeline is the only path.
  systemd.services = lib.mkMerge [
    {}
    # docs/adr/0015-daemon-only-clean-break.md.
    (lib.mkIf false {})
  ];

  # Restrict /dev/kvm to kvm group members only (was world-rwx 0666).
  # Only nixling-<vm>-gpu system users and the microvm service user are in
  # the kvm group after; the host's Wayland user is no longer a member.
  # The /dev/kvm rule is unconditional; Yubico-specific rules are gated on
  # `nixling.site.yubikey.enable` (the option was previously declared but
  # unused).
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
  # nixling-<vm>-{gpu,snd,swtpm} ExecStartPre/Post stanzas)
  environment.systemPackages = [
    pkgs.linuxPackages.usbip
    pkgs.swtpm
    pkgs.tpm2-tools
    pkgs.acl
  ];

  # pre-create /run/nixling and lock file at boot.
  # Lock file is group=nixling so members of that group can
  # open it with `exec 9>` without write access to root:root 0755
  # /run/nixling.
  #
  # `/run/nixling/vms/` is the parent for per-VM RuntimeDirectory= entries
  # (e.g. /run/nixling/vms/<vm>/snd.sock from the audio sidecar). Pre-creating
  # it root:root 0755 means systemd's RuntimeDirectory creation only owns
  # the leaf <vm>/ dir, keeping the shared parent under root ownership.
  # `/run/nixling/alloy/` is a private subtree for observability sockets so
  # Alloy no longer needs write access to the shared launcher/audio lock root.
  # also pre-create the GPU sidecar's runtime root.
  # when daemonExperimental is
  # enabled, /run/nixling is owned exclusively by host-daemon.nix
  # (nixlingd:nixling 0750). This module emits its OWN
  # /run/nixling rule only in the pre-daemon path. The duplicate
  # `if daemonExperimental … 0755 root root` form is REMOVED.
  systemd.tmpfiles.rules = lib.optionals (! cfg.daemonExperimental.enable) [
    "d /run/nixling             0775 root nixling -"
  ] ++ [
    "d /run/nixling/vms         0755 root root -"
    "f /run/nixling/usbipd.lock 0660 root nixling -"
    "d /run/nixling-gpu         0755 root root -"
    # security-r7-2: lock file for nixling-known-hosts-refresh@.service
    # so concurrent refresh runs (one per VM at boot) serialize against the
    # same file the CLI do_trust path uses. Mode 0660 root:nixling
    # so launcher-group members can also flock it from `nixling trust`.
    "f ${cfg.site.stateDir}/known_hosts.nixling.lock 0660 root nixling -"
    # keys directory for nixling-managed SSH keys.
    # Created root:root 0700 — the generator activation script
    # (deferred) will populate it.
    "d ${cfg.site.keysDir} 0710 root nixling -"
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
