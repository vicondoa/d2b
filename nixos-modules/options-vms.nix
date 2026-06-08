# nixling.vms.<vm>.* — per-VM submodule schema. Includes the
# component toggles (graphics.enable / tpm.enable / usbip.* /
# audio.* / audit.*) whose matching files under
# `nixos-modules/components/`
# are conditionally imported by host.nix on this submodule's
# resolved values. Extracted from options.nix in Phase 2c
# (split-options) for reviewability.
{ lib, ... }:

{
  options.nixling.vms = lib.mkOption {
    description = "MicroVMs to declare via the nixling framework.";
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
      # v1.1-P2: options-vms-removed.nix's mkRemovedOptionModule shim
      # was retired here. It cannot be imported into an `attrsOf
      # submodule` per-instance because the per-VM submodule layer
      # does not have an `assertions` option (NixOS assertions live
      # at the top-level config root). The defense-in-depth
      # assertion in `assertions.nix` is the sole supervisor-removal
      # error path; the friendly message text matches the original
      # mkRemovedOptionModule wording verbatim.
      options = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether this microVM is registered with microvm.nix.";
        };
        autostart = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Start this microVM at host boot. Graphics VMs cannot
            autostart (the systemd unit has no Wayland session).
            Net VMs declared via nixling.envs.<env> default
            to autostart = true (set in network.nix).
          '';
        };
        config = lib.mkOption {
          # `deferredModule` lets multiple definitions merge as
          # modules: framework-side `lib.mkDefault { imports = [...]; }`
          # combines with consumer-side `{ users.users.x = ...; }`
          # via the module system's natural attribute merge,
          # instead of one definition stomping the other (which is
          # what `types.unspecified` used to do — see Wave-6
          # consumer-integration findings).
          type = lib.types.deferredModule;
          default = { };
          example = lib.literalExpression "{ imports = [ ../../vms/foo.nix ]; }";
          description = ''
            NixOS module merged into the guest's configuration.
            Typically `{ imports = [ ../../vms/<name>.nix ]; }`.

            Multiple definitions (e.g. one from the framework, one
            from a consumer override) are merged as modules: imports
            are concatenated, attribute paths recursively combined.
          '';
        };

        # v1.1-final: nixling-owned per-VM evaluator output.
        # Populated by host.nix's composeVm pass which runs the
        # consumer's `config` through the nixling-owned per-VM
        # NixOS evaluator (see vm-evaluator.nix). Stored at the
        # SIBLING attribute `nixling._computed.vms.<name>` rather
        # than `nixling.vms.<name>.computed` to avoid the
        # NixOS-module infinite-recursion that occurs when a
        # mapAttrs-over-cfg.vms write target is the same attribute
        # path the iteration reads from.
        # (Retained as an internal placeholder option for
        # compat with consumers that may already touch the path;
        # always empty at the option level.)
        computed = lib.mkOption {
          type = lib.types.unspecified;
          default = { config = { }; options = { }; };
          internal = true;
          description = ''
            DEPRECATED at v1.1-final: per-VM evaluation output moved
            to `nixling._computed.vms.<name>` to avoid module-system
            infinite recursion. Read via `nl.vmRunner config name`
            from `nixos-modules/lib.nix` (helpers route to the new
            location automatically).
          '';
        };

        graphics.enable = lib.mkEnableOption ''
          virtio-gpu + Wayland cross-domain forward to the host
          compositor. Implies hypervisor = cloud-hypervisor, pulls
          in the spectrum-os-patched CH plus the patched crosvm GPU
          sidecar, and auto-launches a foot terminal in the guest
          so the VM has a visible window the moment it boots.
        '';

        graphics.crossDomainTrusted = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Gate the cross-domain Wayland context type in the crosvm GPU
            sidecar. Default false. Set true only for VMs where cross-domain
            forwarding is the primary use case (e.g. a Wayland-forwarding
          launchpad VM running FreeRDP or another remote-desktop client).
            Must be false for any VM running Docker (privileged-container escape
            inside the VM could attack the host compositor via cross-domain).
            Spec: SECURITY-nixling.md C2#4.
          '';
        };

        tpm.enable = lib.mkEnableOption ''
          swtpm 2.0-backed TPM device exposed to the guest as a TPM
          CRB at /dev/tpm0 + /dev/tpmrm0. Implies hypervisor =
          cloud-hypervisor (the only one microvm.nix can wire swtpm
          to). Persistent state lives in
          /var/lib/nixling/vms/<vm>/swtpm/ on the host.
        '';

        usbip.yubikey = lib.mkEnableOption ''
          YubiKey USBIP passthrough. Loads vhci_hcd in the guest and
          installs the usbip CLI so the `nixling usb <vm>` host-side
          wrapper can redirect a plugged-in Yubico device from the
          host's xhci to this VM via USBIP. Host-side daemon is
          per-env (`nixling-sys-<env>-usbipd-proxy.service`); see
          `nixos-modules/network.nix`.
        '';

        usbip.busids = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "1-1.4" ];
          description = ''
            Exact USBIP busids this VM is allowed to claim inside its
            env. Emitted into `host.json.environments[].usbipBusidLocks[]
            .busIds` for the daemon/broker intent resolver. Leave empty
            to preserve the legacy placeholder `pending` fallback used by
            v0.4-era host.json fixtures.
          '';
        };

        audio.enable = lib.mkEnableOption ''
          Host microphone + speaker, mediated via vhost-user-sound +
          PipeWire. Setting this only enables the *capability*: the
          per-VM state file at
          /var/lib/nixling/vms/<vm>/state/audio-state.json is what
          actually decides whether the VM has a virtio-sound device
          at any given moment. Both mic and speaker default to OFF
          on first materialisation (unless allowMicByDefault /
          allowSpeakerByDefault below are flipped). Live grant/revoke
          is via `nixling audio …`. The VM shows up in plasma-pa as a
          PipeWire client named `nixling-<vm>` for per-stream mute /
          volume.
        '';

        audio.allowMicByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Initial value of the `mic` field when the per-VM audio
            state file is first materialised. Only consulted at file
            creation time; subsequent edits via `nixling audio …`
            persist. Default false (explicit-grant model).
          '';
        };

        audio.allowSpeakerByDefault = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Initial value of the `speaker` field when the per-VM
            audio state file is first materialised. Only consulted at
            file creation time; subsequent edits via `nixling audio
            …` persist. Default false (explicit-grant model).
          '';
        };

        audio.users = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "alice" ];
          description = ''
            Guest-side usernames that should be added to the `audio`
            group inside the VM. The virtio-snd ALSA driver exposes
            `/dev/snd/*` with mode `0660 root:audio`, so an
            interactive user wanting to talk to WirePlumber /
            PipeWire from a non-logind-active session needs the
            group explicitly.

            Defaults to `[ ssh.user ]` when `ssh.user` is set,
            otherwise the empty list. Override here if the VM has
            additional interactive users.
          '';
        };

        audit = {
          enable = lib.mkEnableOption ''
            guest-side auditd with forwarding to the existing
            observability pipeline (guest Alloy → vsock → Loki)
          '';

          rules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [
              "-w /etc/passwd -p wa -k identity"
              "-w /etc/shadow -p wa -k identity"
              "-w /etc/sudoers -p wa -k priv-esc"
            ];
            description = ''
              Curated guest-side audit rules. Propagated to the
              guest's `nixling.audit.rules` when `audit.enable = true`.
              The default excludes `execve` argv capture because
              command lines routinely carry secrets; add that rule
              explicitly only for short-lived, high-sensitivity audits.
            '';
          };
        };

        observability.enable = lib.mkEnableOption ''
          guest Alloy agent + reverse OTLP tunnel from the
          observability stack VM
        '';

        observability.scrapeJournal = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Whether the future observability guest component should
            scrape this VM's journald stream.
          '';
        };

        observability.scrapeNodeMetrics = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = ''
            Whether the future observability guest component should
            scrape this VM's node/system metrics.
          '';
        };

        env = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "work";
          description = ''
            Name of a `nixling.envs.<env>` this VM belongs to.
            When non-null, the framework auto-derives the VM's MAC
            and IP from `(env, index)`, creates a tap on the env's
            LAN bridge, and registers a dnsmasq host-reservation on
            the env's net VM. The VM's guest-side network config
            becomes pure DHCP.

            null = legacy mode (must hand-roll microvm.interfaces +
            systemd.network in the guest). Net new VMs should set
            env.
          '';
        };

        index = lib.mkOption {
          type = lib.types.ints.between 10 250;
          default = 10;
          description = ''
            Workload-VM IP index within its env's LAN subnet. The
            VM's IP = <lan-subnet-prefix>.<index>. Range 10–250 to
            leave room for the net VM (.1), gateway-ish reservations
            (.2–.9), and DHCP pool overflow (.251–.254). Must be
            unique within an env.
          '';
        };

        # DEPRECATED. Pre-env hint used by the CLI to know the VM's
        # IP. With `env` set, the framework derives the IP and the
        # CLI reads it from the same source — don't set this.
        staticIp = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "10.10.0.10";
          description = ''
            DEPRECATED. Use `env` + `index` instead — those derive
            the IP and propagate it everywhere (CLI manifest,
            dnsmasq reservation, guest networkd). Setting both
            `staticIp` and `env` is an error.

            null = no static IP and no env = the CLI cannot ssh.
          '';
        };

        ssh = {
          user = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "alice";
            description = "Username for `nixling`-driven SSH into the VM.";
          };
          keyPath = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            defaultText = lib.literalExpression
              "\"\${config.nixling.site.keysDir}/<vm>_ed25519\"";
            example = "~/.ssh/example-vm_ed25519";
            description = ''
              Private-key path for `nixling`-driven SSH into the VM.

              **Default (when unset): derived from
              `nixling.site.keysDir`** as
              `<keysDir>/<vm>_ed25519`, matching the framework-
              managed Ed25519 key generated by `host-keys.nix` on
              every activation. The derived default is applied at
              module-merge time in `host-keys.nix`, so the apparent
              `null` here just means "let the framework decide".

              Override only if you supply your own per-VM key (e.g.
              a hardware-backed key whose private half does not
              live in `<keysDir>/`). The framework-managed key
              itself is still generated regardless; see
              `nixling.site.keysDir`.

              Setting this to `null` AND `nixling.site.keysDir` to
              an unreadable location is the only way to opt out of
              the framework-managed key entirely — `nixling` CLI
              subcommands that need SSH (`keys rotate`, `switch`,
              …) will refuse to run when the resolved path doesn't
              exist.
            '';
          };
        };

        sudo = lib.mkEnableOption ''
          passwordless sudo for the VM's SSH user (`ssh.user`). When
          enabled, the framework adds a NOPASSWD sudoers rule for
          the user inside the guest, allowing `sudo` from the
          nixling SSH session without an interactive password prompt.
          Useful for development/debugging VMs where the SSH user
          needs root for `tpm2_flushcontext`, `systemctl restart`,
          etc.
        '';

        userAuthorizedKeys = lib.mkOption {
          type = lib.types.listOf
            (lib.types.oneOf [ lib.types.path lib.types.str ]);
          default = [ ];
          example = lib.literalExpression ''
            [ ./keys/alice_id_ed25519.pub ]
          '';
          description = ''
            Per-VM authorized SSH keys, merged with the global
            `nixling.site.userAuthorizedKeys` set. Both lists are
            handed to the VM's `nixling-load-host-keys.service`,
            which writes them — together with the framework's own
            nixling-managed pubkey for this VM — into the SSH user's
            `authorized_keys` file.

            Entries follow the same shape as
            `nixling.site.userAuthorizedKeys`: paths to `.pub` files
            or literal pubkey strings, validated at eval time.
          '';
        };

        homeManager = {
          enable = lib.mkEnableOption ''
            Home Manager inside this VM. Imports
            `nixos-modules/components/home-manager.nix` which pulls in
            the upstream HM NixOS module and applies sensible defaults
            (useGlobalPkgs, useUserPackages, .hm-backup, inputs in
            extraSpecialArgs).
          '';
          users = lib.mkOption {
            type = lib.types.attrsOf lib.types.unspecified;
            default = { };
            description = ''
              Per-user HM config. See
              `nixos-modules/components/home-manager.nix` for the
              expected shape.
            '';
          };
        };

        # REMOVED in Phase 2b. The submodule path is kept ONLY so that
        # legacy consumer assignments produce a readable assertion
        # error rather than the cryptic "option does not exist"
        # message the module system would emit if we simply dropped
        # the option. The stub is `internal = true` so it does not
        # appear in generated docs.
        #
        # Migration: see CHANGELOG.md (Phase 2b: Removed) and the
        # `vicondoa/nixos-entra-id` flake README for the new
        # composition pattern (per-VM config.imports of the sibling
        # flake's nixosModules.default).
        entra-id = lib.mkOption {
          type = lib.types.attrsOf lib.types.unspecified;
          default = { };
          internal = true;
          visible = false;
          description = ''
            REMOVED in Phase 2b. Use `vicondoa/nixos-entra-id`'s
            module per-VM via `nixling.vms.<vm>.config.imports`.
          '';
        };
      };
    }));
  };

  # v1.1-final: per-VM evaluation outputs stored OUTSIDE
  # `nixling.vms.<name>` to avoid module-system infinite recursion.
  # host.nix's composeVm pass populates `nixling._computed.vms.<name>`
  # with the evaluated NixOS attrset (config + options) for each
  # enabled VM. lib.nix helpers (vmRunner / vmToplevel /
  # vmDeclaredRunner) route through this attribute.
  options.nixling._computed = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    visible = false;
    description = ''
      Internal storage for per-VM evaluator outputs. Populated by
      host.nix's composeVm pass. Stored here (not under
      `nixling.vms.<name>.computed`) to avoid the NixOS module-
      system infinite-recursion that occurs when a mapAttrs over
      cfg.vms writes back to the same `nixling.vms` attribute it
      reads from.
    '';
  };
}
