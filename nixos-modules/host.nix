{ inputs }:

{ config, pkgs, lib, ... }:

let
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) subnetIp mkMac;

  cfg = config.nixling;
  envMeta = cfg._envMeta;

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

  # Stable per-VM AF_VSOCK CID, derived from md5(vmName). Required
  # for the cloud-hypervisor backend to wire systemd-notify back to
  # the host's `microvm@<name>.service` (vsock is the only transport
  # CH supports for sd_notify). Setting it for every VM also silences
  # microvm.nix's eval-time
  #   "cloud-hypervisor supports systemd-notify via vsock, but
  #    microvm.vsock.cid must be set to enable this"
  # warning. CIDs 0/1/2 are reserved (hypervisor/local/host); we take
  # 24 bits of md5 (range 0..16_777_215) and offset by 4096 to stay
  # clear of the reserved range and any low CIDs other host tooling
  # might grab. Collision-resistant for ~1k VMs (birthday bound on
  # 2^24 is ~4k).
  vmVsockCid = name:
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
    ./host-known-hosts.nix
    ./host-audit.nix
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
      in {
        specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;
        config = lib.mkMerge [
          {
            imports = [ ./base.nix ]
              ++ lib.optional vm'.graphics.enable ./components/graphics.nix
              ++ lib.optional vm'.tpm.enable ./components/tpm.nix
              ++ lib.optional vm'.usbip.yubikey ./components/usbip.nix
              ++ lib.optional vm'.audio.enable ./components/audio/guest.nix
              # Note: Entra ID / Himmelblau is NOT a nixling component.
              # Consumers who need it import it per-VM via:
              #   nixling.vms.<vm>.config.imports = [
              #     inputs.nixos-entra-id.nixosModules.default
              #   ];
              # See `vicondoa/nixos-entra-id` for the sibling flake.
              ++ lib.optional vm'.homeManager.enable ./components/home-manager.nix
              ++ lib.optional (derived != null) (envWorkloadGuestModule derived)
              ++ [ vm'.config ];

            # Per-VM vsock CID (see vmVsockCid above). mkDefault so a
            # VM file can pin its own if it ever needs to (e.g. a
            # guest service that depends on a fixed CID).
            microvm.vsock.cid = lib.mkDefault (vmVsockCid name);
          }
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
          # Propagate the SSH user to the guest so
          # nixling-load-host-keys.service knows whose
          # authorized_keys to populate (Phase 2b nixling-managed keys).
          { nixling.sshUser = vm'.ssh.user; }
        ];
      })
    (lib.filterAttrs (_: vm: vm.enable) cfg.vms);

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
  ];
}
