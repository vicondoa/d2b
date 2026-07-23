{ inputs }:

{ config, pkgs, lib, ... }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) subnetIp mkMac;
  # Per-VM evaluator entry point. composeVm is the d2b-owned
  # replacement for microvm.nix's per-VM
  # lib.evalModules invocation; see vm-evaluator.nix +
  # vm-options.nix. host.nix's `d2b._computed` mapping below
  # calls composeVm for every enabled VM and stores the result
  # at `config.d2b._computed.<name>.config.*`, where the
  # lib.nix helpers (vmRunner / vmToplevel / vmDeclaredRunner)
  # read it.
  vmSubmodule = (import ./vm-submodule.nix { inherit inputs; })
    { inherit config lib pkgs; };
  composeVm = vmSubmodule._composeVm;

  cfg = config.d2b;
  index = cfg._index;
  envMeta = index.envMeta;
  obsCfg = cfg.observability;
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
      d2b.vms.${name}: graphics/audio components are
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

  enabledVms = index.enabledVms;
  normalNixosVms' = index.normalNixosVms;
  # `d2b.vms.<vm>.supervisor` was removed per ADR 0015.
  # Every enabled VM is now daemon-supervised; the systemd-template
  # path is retired. The empty `systemdSupervisedVms` set keeps
  # legacy iteration sites from emitting per-VM microvm@<vm>
  # template instances; the v1.1- phase deletes those sites
  # outright when the template definitions themselves go.
  systemdSupervisedVms = { };
  daemonSupervisedVmNames = index.normalNixosVmNames;

  usbipYubikeyVmEnabled = index.usbip.vmNames != [ ];

  workloadObsVmNames = index.components.observability.vmNames;

  # `transport-vsock` lands before `observability-vm.nix`, so don't
  # auto-start relay instances until the auto-declared obs VM actually
  # exists in `cfg.vms`.
  obsVmEnabled = index.observability.stackVmEnabled;

  relayVmNames =
    if obsVmEnabled
    then workloadObsVmNames
    else [ ];

  # Graphics VMs run via d2b-<vm>-gpu.service (the GPU sidecar IS
  # the CH runner — they bypass microvm@). Headless VMs use microvm@.
  # Both runners need to start the per-VM relay so the host-side vsock
  # bridge actually connects. Templated systemd units can't have
  # per-instance BindsTo/Wants, so we wire per-VM `wants` on each
  # graphics gpu sidecar separately below.
  graphicsRelayVmNames =
    lib.filter (name: (cfg.vms.${name}.graphics.enable or false)) relayVmNames;

  relayEligibilityScript = pkgs.writeShellScript "d2b-otel-relay-eligible" ''
    case " ${lib.concatStringsSep " " relayVmNames} " in
      *" $1 "*) exit 0 ;;
      *) exit 1 ;;
    esac
  '';

  baseVsock = name: {
    cid = config.d2b.manifest.${name}.observability.vsockCid;
    socket = config.d2b.manifest.${name}.observability.vsockHostSocket;
  };

in

{
  imports = [
    # inputs.microvm.nixosModules.host import REMOVED.
    # The d2b-owned per-VM evaluator (vm-evaluator.nix +
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

  # Per-VM state directory layout. /var/lib/d2b/vms/<vm>/ carries
  # every d2b-managed file for the VM (no more
  # microvm.nix-driven /var/lib/microvms/<vm>/ path; d2b owns
  # the substrate end-to-end). The `microvm.stateDir`,
  # `microvm.autostart`, and `systemd.targets.microvms.wants`
  # assignments that lived here under v1.0 are deleted with the
  # upstream microvm.nix host module import.

  # Per-VM NixOS evaluation lives on the d2b-owned
  # `d2b._computed.<name>` attribute (see vm-evaluator.nix).
  # Populated via the composeVm closure imported at the top of this
  # module. Storage location is `d2b._computed` (sibling to
  # `d2b.vms`) rather than `d2b.vms.<name>.computed` to
  # avoid module-system infinite recursion (a mapAttrs over
  # cfg.vms cannot write back to the same d2b.vms attribute
  # path it reads from).
  d2b._computed = lib.mapAttrs
    (name: vm:
      let
        vm' = checkVmPlatform name vm;
        derived = vmDerive name vm';
        chVsock = baseVsock name;
        obsCfg = cfg.observability;
        isObsVm = obsCfg.enable && name == obsCfg.vmName;
        obsSecretsShare = lib.optional isObsVm {
          source = "${cfg.site.stateDir}/observability";
          mountPoint = "/run/d2b-obs-secrets";
          tag = "d2b-obs-sec";
          proto = "virtiofs";
        };
        guestControlShare = lib.optional vm'.guest.control.enable {
          source = "${cfg.site.stateDir}/guest-control-${name}";
          mountPoint = "/run/d2b-guest-control-host";
          tag = "d2b-gctl";
          proto = "virtiofs";
          readOnly = true;
        };
        composedModules = [
          # Framework guest baseline + d2b-managed sshd host keys.
          ./base.nix
          ./guest-sshd-host-keys.nix
        ]
          ++ lib.optional vm'.graphics.enable ./components/graphics.nix
          ++ lib.optional vm'.tpm.enable ./components/tpm.nix
          ++ lib.optional vm'.usbip.yubikey ./components/usbip.nix
          ++ lib.optional vm'.usb.securityKey.enable ./components/security-key-guest.nix
          ++ lib.optional vm'.audio.enable ./components/audio/guest.nix
          ++ lib.optional vm'.audit.enable ./components/audit.nix
          ++ lib.optional (vm'.graphics.enable && vm'.graphics.videoSidecar) ./components/video/guest.nix
          ++ lib.optional vm'.observability.enable ./components/observability/guest.nix
          ++ lib.optional vm'.homeManager.enable ./components/home-manager.nix
          ++ lib.optional (derived != null) (envWorkloadGuestModule derived)
          ++ [ vm'.config ]
          ++ lib.optional (vm'.guestConfigFile != null) vm'.guestConfigFile
          # Seed the guest-editable config INTO the VM so an operator can
          # see + edit it from inside the guest, then `d2b config
          # sync` it back. The read-only baseline always reflects the
          # currently-approved guestConfigFile; a writable working copy
          # is seeded once (tmpfiles `C` = copy-if-absent) for the SSH
          # user to edit. No new host surface — it rides the normal
          # read-only closure (no virtiofs share).
          ++ lib.optional (vm'.guestConfigFile != null) (
            { lib, ... }: let
              # D17/D18: the operator-editable working copy is part of
              # the GuestConfig target and exists independently of any
              # SSH metadata. `ssh.user` only chooses ownership when it
              # is set; when absent the framework defaults ownership to
              # root (guestd reads the copy, and the guest-control exec
              # path edits it, as root).
              owner = if vm'.ssh.user != null then vm'.ssh.user else "root";
            in {
              environment.etc."d2b/guest-config.nix".source = vm'.guestConfigFile;
              systemd.tmpfiles.rules = [
                "d /var/lib/d2b-guest 0750 ${owner} users -"
                "C /var/lib/d2b-guest/guest-config.nix 0640 ${owner} users - /etc/d2b/guest-config.nix"
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
            d2b.homeManager.users = vm'.homeManager.users;
          }
          ++ lib.optional vm'.graphics.enable {
            d2b.graphics.crossDomainTrusted = vm'.graphics.crossDomainTrusted;
            d2b.graphics.xwayland.enable = vm'.graphics.xwayland.enable;
            d2b.graphics.virglVideo = vm'.graphics.virglVideo;
          }
          ++ lib.optional vm'.audio.enable {
            d2b.audio.users =
              if vm'.audio.users != [ ]
              then vm'.audio.users
              else lib.optional (vm'.ssh.user != null) vm'.ssh.user;
          }
          ++ lib.optional vm'.audit.enable {
            d2b.audit.enable = true;
            d2b.audit.rules = vm'.audit.rules;
          }
          ++ lib.optional vm'.observability.enable {
            d2b.observability.scrapeJournal = vm'.observability.scrapeJournal;
            d2b.observability.scrapeNodeMetrics = vm'.observability.scrapeNodeMetrics;
            d2b.observability.identity.vmName = name;
            d2b.observability.identity.envName = if vm'.env != null then vm'.env else "none";
          }
          ++ [
            {
              d2b.sshUser = vm'.ssh.user;
              d2b.sudo = vm'.sudo;
              d2b.guestControl.enable = vm'.guest.control.enable;
              # D17: thread the operator-editable working-copy path into
              # the guest independently of ssh.user so guestd advertises
              # the ReadGuestFile capability exactly when there is a
              # guestConfigFile to sync.
              d2b.guestControl.guestConfigPath =
                if vm'.guestConfigFile != null
                then "/var/lib/d2b-guest/guest-config.nix"
                else null;
              d2b.guestControl.exec = {
                enable = lib.mkForce vm'.guest.exec.enable;
                # The host-fixed workload user every exec runs as (never root),
                # derived from the per-VM workload user. guestd runs every exec
                # as this user in a PAM login session.
                execUser = lib.mkForce vm'.ssh.user;
                detachedMaxRuntimeSec = lib.mkForce vm'.guest.exec.detachedMaxRuntimeSec;
                interactiveMaxRuntimeSec = lib.mkForce vm'.guest.exec.interactiveMaxRuntimeSec;
              };
              d2b.guestControl.shell = {
                enable = lib.mkForce vm'.guest.shell.enable;
                defaultName = lib.mkForce vm'.guest.shell.defaultName;
                maxSessions = lib.mkForce vm'.guest.shell.maxSessions;
                maxAttached = lib.mkForce vm'.guest.shell.maxAttached;
              };
            }
            # Per-VM framework-managed shares moved from store.nix to
            # break the module-system infinite
            # recursion store.nix would cause when mapping over
            # cfg.vms to write back to d2b.vms).
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
                  # Guest metadata share is rooted at the signed
                  # `store-view/meta` subtree (ADR 0027), NOT the
                  # store-view root: the root also holds the served
                  # `live/` hardlink pool and the host-only
                  # `state/`, `gcroots/`, and `sync.lock` which must
                  # never reach the guest. virtiofsd serves this share
                  # `--readonly` (forced in processes-json.nix off the
                  # `d2b-meta` tag, independent of this source path).
                  source = "${cfg.store.stateDir}/${name}/store-view/meta";
                  mountPoint = "/run/d2b-store-meta";
                  tag = "d2b-meta";
                  proto = "virtiofs";
                }
                {
                  source = "${cfg.site.stateDir}/vms/${name}/host-keys";
                  mountPoint = "/run/d2b-host-keys";
                  tag = "d2b-hkeys";
                  proto = "virtiofs";
                }
                {
                  source = "${cfg.site.stateDir}/vms/${name}/sshd-host-keys";
                  mountPoint = "/run/d2b-sshd-host-keys";
                  tag = "d2b-ssh-host";
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
          else d2bLib.guestConfigForbiddenNamespaces
            {
              inherit pkgs;
              specialArgs = { inherit inputs; name = name; } // cfg.site.extraSpecialArgs;
            }
            vm'.guestConfigFile;
      })
    normalNixosVms';

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
  # Only d2b-<vm>-gpu system users and the microvm service user are in
  # the kvm group after; the host's Wayland user is no longer a member.
  # The /dev/kvm rule is unconditional; Yubico-specific rules are gated on
  # `d2b.site.yubikey.enable` (the option was previously declared but
  # unused).
  services.udev.extraRules = ''
    # H5 — lock KVM device to kvm group, no longer world-accessible
    KERNEL=="kvm", GROUP="kvm", MODE="0660"
  '' + lib.optionalString config.d2b.site.yubikey.enable ''
    # Yubico YubiKey — hidraw (FIDO/U2F)
    SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1050", GROUP="kvm", MODE="0660", TAG+="uaccess"
    # Yubico YubiKey — raw USB device
    SUBSYSTEM=="usb", ATTRS{idVendor}=="1050", GROUP="kvm", MODE="0660", TAG+="uaccess"
  '';

  # Host modules required by d2b's Cloud Hypervisor + virtiofs substrate.
  # These may be built in on some kernels; when they are loadable modules,
  # preload them before d2bd's startup contract check runs. USBIP remains
  # conditional on the host/VM YubiKey gate.
  boot.kernelModules = [
    "vhost_net"
    "tun"
    "virtio_blk"
    "virtio_console"
    "virtio_net"
    "virtio_pci"
    "virtiofs"
  ] ++ lib.optionals (cfg.site.yubikey.enable && usbipYubikeyVmEnabled) [
    "usbip-host"
  ];

  # Host-side debug binaries + acl (setfacl used in activation scripts and
  # d2b-<vm>-{gpu,snd,swtpm} ExecStartPre/Post stanzas)
  environment.systemPackages = [
    pkgs.linuxPackages.usbip
    pkgs.swtpm
    pkgs.tpm2-tools
    pkgs.acl
  ];

  # pre-create /run/d2b and lock file at boot.
  # Lock file is group=d2b so members of that group can
  # open it with `exec 9>` without write access to root:root 0755
  # /run/d2b.
  #
  # `/run/d2b/vms/` is the parent for per-VM runtime sockets. In the
  # daemon path host-activation.nix owns its tmpfiles posture
  # (root:d2b 0750 parent, d2bd:d2b per-VM leaves) so broker
  # path-safety can create/reconcile children without trusting a
  # daemon-writable parent. The legacy pre-daemon path keeps the old
  # root-owned 0755 parent for retired RuntimeDirectory users.
  # `/run/d2b/alloy/` is a private subtree for observability sockets so
  # Alloy no longer needs write access to the shared launcher/audio lock root.
  # Also pre-create the legacy GPU sidecar runtime root in the pre-daemon path;
  # daemon-native runtime roots are owned by host-activation.nix tmpfiles.
  # when daemonExperimental is
  # enabled, /run/d2b is owned exclusively by host-daemon.nix
  # (root:d2b 1770 with explicit ACLs). This module emits its OWN
  # /run/d2b rule only in the pre-daemon path. The duplicate
  # `if daemonExperimental … 0755 root root` form is REMOVED.
  systemd.tmpfiles.rules = lib.optionals (! cfg.daemonExperimental.enable) [
    "d /run/d2b             0775 root d2b -"
  ] ++ lib.optionals (! cfg.daemonExperimental.enable) [
    "d /run/d2b/vms         0755 root root -"
    "d /run/d2b-gpu         0755 root root -"
  ] ++ [
    "d /run/d2b/otel        0750 d2bd d2b -"
    "f /run/d2b/usbipd.lock 0660 root d2b -"
    # security-r7-2: lock file for d2b-known-hosts-refresh@.service
    # so concurrent refresh runs (one per VM at boot) serialize against the
    # same file the CLI do_trust path uses. Mode 0660 root:d2b
    # so launcher-group members can also flock it from `d2b trust`.
    "f ${cfg.site.stateDir}/known_hosts.d2b.lock 0660 root d2b -"
    # keys directory for d2b-managed SSH keys.
    # Created root:root 0700 — the generator activation script
    # (deferred) will populate it.
    "d ${cfg.site.keysDir} 0710 root d2b -"
    "D ${cfg.site.tmpDir} 0755 root root -"
  ]
  # /run/d2b/alloy is created at service-start time by
  # alloy.service's `RuntimeDirectory=d2b/alloy` directive, not
  # via tmpfiles — the alloy account is a systemd DynamicUser whose
  # UID/GID is only allocated at start, and tmpfiles cannot chown
  # to that user at activation time. d2b-otel-host-bridge's
  # ExecStartPre setfacl runs AFTER alloy.service (After= + bindsTo)
  # so the directory is guaranteed to exist by then.
  # Per-VM state roots are postured by host-activation.nix tmpfiles under
  # the daemon-native ownership matrix. Do not reintroduce the retired
  # microvm:kvm tmpfiles shape here.
  ;
}
