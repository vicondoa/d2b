# nixling.site.* — host-level site knobs that every VM inherits,
# plus the top-level `nixling.hostLanCidrs` list. Extracted from
# options.nix in Phase 2c (split-options) for reviewability.
{ lib, ... }:

{
  # Site-specific knobs (Phase 2b — extracted from previously-hard-
  # coded references to the maintainer's host setup). Every option
  # here is opt-in: leaving the defaults gives you a fully headless
  # framework with no Wayland integration and no nixling-managed SSH
  # keys, which is exactly what consumers running headless / CI / pure-
  # net VMs want. Graphics or audio VMs require `waylandUser`.
  options.nixling.site = {
    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nixling";
      example = "/var/lib/nixling";
      description = ''
        Root of every nixling-managed state file on the host. Per-VM
        state lives under `${"$"}{stateDir}/vms/<vm>/`; nixling-
        managed SSH keys under `${"$"}{stateDir}/keys/`. Must be on
        the same filesystem as `/nix/store` for the per-VM hardlink
        farm to work (see `nixling.store.stateDir`, which defaults to
        `${"$"}{stateDir}/vms`).

        **ADVISORY ONLY in v0.1.0.** The framework hardcodes
        `/var/lib/nixling` in several places (host.nix's
        `microvm.stateDir`, host-activation.nix's ACL repair,
        host-known-hosts.nix's known_hosts location, host-sidecars.nix's
        per-VM unit WorkingDirectory + ExecStart, and the per-VM
        manifest baked by cli.nix). Overriding this option in v0.1.0
        WILL break those code paths. Full threading lands in v0.2.0
        (`med-findings-postrelease`: `stateDir-threading`); until
        then, leave it at the default. The option is declared so
        future docs / overrides have a stable name and so consumers
        can read the framework's nominal state-root without
        grepping.
      '';
    };

    waylandUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "alice";
      description = ''
        Username of the host's primary Wayland session user. The GPU
        + audio sidecars bind this user's `/run/user/<uid>/wayland-0`
        and `/run/user/<uid>/pipewire-0` into their private mount
        namespaces, so a non-null value is required for any VM that
        sets `graphics.enable = true` or `audio.enable = true`.

        Leave at `null` for headless deployments. Eval fails with a
        clear message if a graphics or audio VM is declared without
        this option set.

        The user is also expected to be one of `launcherUsers` (so
        the per-VM sidecar polkit grant applies on click), but that
        is enforced separately and not a hard requirement here.
      '';
    };

    launcherUsers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = ''
        Users to add to the `nixling-launcher` group. Members of that
        group get a polkit grant to run start/stop/restart against
        the framework's own systemd units without a password prompt
        (see `nixos-modules/host-polkit.nix` for the exact-unit
        allowlist).

        The framework does NOT create the users — declare them in
        your top-level NixOS config with `users.users.<name> = { …
        };`. nixling only adds the `nixling-launcher` group to their
        `extraGroups`.

        Empty list = nobody is a launcher principal. The framework
        still works (sudo + polkit-password prompts cover everything
        the launcher group's allowlist grants).
      '';
    };

    keysDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nixling/keys";
      example = "/var/lib/nixling/keys";
      description = ''
        Directory where the framework generates and stores
        per-VM SSH host keys. Mode 0700 owned by root, with a per-
        key ACL granting read access to the `nixling-launcher` group
        (so the CLI can drive `ssh` to each VM without sudo).

        Default tracks `${"$"}{stateDir}/keys`. If you override
        `stateDir`, override this too — the option default is a
        literal path because Nix evaluates option defaults
        independently of other options.

        **ADVISORY ONLY in v0.1.0** (same caveat as `stateDir`).
        host-keys.nix's tmpfiles rules and activation script DO
        thread `cfg.site.keysDir`, but host.nix's tmpfiles rule
        currently re-declares the literal `/var/lib/nixling/keys`,
        and the migration script under `scripts/` hardcodes the
        same path. Overriding this option in v0.1.0 will leave
        those stale entries on disk; the per-VM key flow itself
        still works because everything goes through host-keys.nix.
        Full alignment lands in v0.2.0
        (`med-findings-postrelease`: `keysDir-threading`).
      '';
    };

    userAuthorizedKeys = lib.mkOption {
      type = lib.types.listOf (lib.types.oneOf [ lib.types.path lib.types.str ]);
      default = [ ];
      example = lib.literalExpression ''
        [
          ./keys/alice_id_ed25519.pub
          "ssh-ed25519 AAAAC3Nz... alice@laptop"
        ]
      '';
      description = ''
        Extra SSH public keys to authorize for the SSH user inside
        every nixling-managed VM. Entries may be either paths to a
        `.pub` file or literal pubkey strings.

        These are merged with the framework's own per-VM
        nixling-managed pubkey when the guest-side
        `nixling-load-host-keys.service` populates the SSH user's
        `authorized_keys` file. Empty list = only the framework's
        own pubkey is authorized.

        Eval fails if any entry doesn't look like a supported pubkey
        type (ed25519, RSA, ECDSA, security-key variants) or contains
        a `-----BEGIN ... PRIVATE KEY-----` marker.
      '';
    };

    yubikey.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      example = false;
      description = ''
        Install host-side Yubikey support: the udev rules for vendor
        ID 1050 (so hidraw / raw-USB nodes carry `GROUP="kvm"
        MODE="0660" uaccess`) AND the `usbip-host` kernel module
        (so `nixling usb <vm>` can re-bind the device into a guest
        via USBIP).

        Set to `false` on hosts that do not use Yubikeys. With this
        option off the framework does not load `usbip-host` and does
        not emit Yubico udev rules; any per-VM `usbip.yubikey = true`
        flag still pulls in the guest-side `usbip` CLI + `vhci_hcd`
        module, but the host side has no Yubikey-specific
        machinery installed. The `/dev/kvm` udev rule (locking the
        device to `GROUP="kvm"`) stays in place regardless — it is
        not a Yubikey-specific rule.
      '';
    };

    flakePath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/etc/nixos";
      description = ''
        Default flake path the `nixling` CLI uses for per-VM
        lifecycle subcommands (`build`, `switch`, `boot`, `test`).
        Each invocation resolves a flake reference of the form
        `<flakePath>#nixling-<vm>` to build the VM's closure.

        Leave null for users who always pass `NIXLING_FLAKE` /
        `--flake` explicitly. Setting it makes
        `nixling switch <vm>` work without arguments on the
        consumer's primary nixos configuration.
      '';
    };

    extraSpecialArgs = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = { };
      example = lib.literalExpression ''
        # Pass the consumer flake's `inputs` to every per-VM module
        # so VMs can reference `inputs.<consumer-input>.*`. Mirrors
        # home-manager's `extraSpecialArgs` pattern.
        { inherit inputs; }
      '';
      description = ''
        Extra module-arguments merged into every per-VM
        `microvm.vms.<vm>.specialArgs` after the framework's own
        baseline (`{ inherit inputs; }` where `inputs` is the
        nixling FLAKE's inputs). Consumer keys take precedence on
        collision — set `inputs = consumerInputs;` here if your
        per-VM modules need `inputs.<your-flake>` visibility (e.g.
        `inputs.nixos-entra-id`, `inputs.llm-agents`).

        Use this when:
        - A per-VM module file (e.g. `vms/work.nix`) takes
          `{ inputs, ... }:` and references inputs your consumer
          flake declares but nixling's flake does not.
        - You want to thread a consumer-side overlay set (e.g.
          `{ myOverlay = inputs.something.overlays.default; }`)
          into per-VM evals without re-importing it in each VM.

        Mirrors `home-manager.extraSpecialArgs` from the
        Home-Manager NixOS module — same semantics, same intent.
      '';
    };
  };

  # Top-level option: CIDRs of the host's own physical LAN(s). These
  # get unioned into every `nixling.envs.<env>.hostBlocklist`
  # automatically, so a workload VM cannot reach any host on the
  # wire the host itself sits on — not just the host's own IP.
  #
  # Defaults to the empty list; override to your actual subnet.
  # `ip route` on the host will tell you what to put here, e.g.
  # `192.168.1.0/24` for a typical home LAN with the host at
  # `192.168.1.42/24`.
  options.nixling.hostLanCidrs = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [ ];
    example = [ "192.168.1.0/24" "10.0.0.0/24" ];
    description = ''
      CIDRs of the host's own physical LAN(s). Automatically merged
      into every env's net-VM DROP rule so VMs cannot reach the
      host's neighbours (printer, NAS, other workstations…) — not
      just the host's IP.

      Same-env workload VMs share an env (and its `sys-<env>-net`
      net VM) but cannot directly reach peer workload VMs —
      workload taps are `Isolated = true` in the per-env LAN
      bridge. Traffic to peers and to the host's LAN both leave
      via the net VM (where the merged DROP rules apply); there
      is no east-west bypass.
    '';
  };
}
