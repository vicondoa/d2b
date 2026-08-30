# d2b.site.* - host-level site defaults plus the top-level
# `d2b.hostLanCidrs` list.
{ lib, ... }:

{
  # Site-specific knobs extracted from previously-hard-
  # coded references to the maintainer's host setup). Every option
  # here is opt-in: leaving the defaults gives you a fully headless
  # framework with no Wayland integration and no d2b-managed SSH
  # keys, which is exactly what consumers running headless / CI / pure-
  # net VMs want. Graphics or audio VMs require `waylandUser`.
  options.d2b.site = {
    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/d2b";
      example = "/var/lib/d2b";
      description = ''
        Root of every d2b-managed state file on the host. Zone
        runtime and audit state is anchored below this directory.

        **Reserved in v0.4.0.** The framework still hardcodes
        `/var/lib/d2b` in several host-side paths, so eval now
        rejects overrides until full threading lands. Leave this at the
        default for now; the option exists so consumers and future
        migrations have a stable name for the framework's nominal
        state root.
      '';
    };

    tmpDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/d2b/tmp";
      readOnly = true;
      description = ''
        Ephemeral state directory, cleaned on every boot via a host
        `systemd-tmpfiles` `D` rule.
        Components SHOULD use `${"$"}{tmpDir}/<vm>/` for any state
        that is safe to lose across reboots (transient sockets,
        temporary swtpm proxies, build artifacts, etc.).
      '';
    };

    allowUnsafeEastWest = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Acknowledge that enabling
        `Network.spec.isolation.allowEastWest` is an explicit
        out-of-threat-model mode. Leave this at `false` to preserve
        the default peer-Guest isolation boundary.
      '';
    };

    usePrebuiltHostTools = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Use release prebuilt host binaries for `d2b`, `d2bd`,
        `d2b-broker`, and the activation helper when they are
        available. Set to `false` on
        development hosts that intentionally validate the checked-out flake's
        Rust sources before a release artifact exists.
      '';
    };

    ch = {
      netHandoffMode = lib.mkOption {
        type = lib.types.enum [ "tap-fd" "persistent-tap" ];
        default = "tap-fd";
        example = "persistent-tap";
        description = ''
          Cloud Hypervisor net-handoff mode for long-lived runners.
          The selected runtime Provider records this value; the broker's
          host check probes the packaged CH binary and fails closed
          with `ch-net-handoff-not-supported` if neither mode
          satisfies the declared VM network resources without
          `CAP_NET_ADMIN` in the long-lived runner.

          - `"tap-fd"` (default): the broker opens TAP +
            `/dev/vhost-net` and passes them via `SCM_RIGHTS`; the
            runner has no `CAP_NET_ADMIN`.
          - `"persistent-tap"` (fallback): the broker creates a
            persistent TAP via `TUNSETOWNER`/`TUNSETGROUP` for the
            runner uid/gid; the runner mounts the device node
            read-only.
        '';
      };
    };

    audit = {
      retentionDays = lib.mkOption {
        type = lib.types.int;
        default = 14;
        example = 30;
        description = ''
          How many days of daily-rotated broker audit log files
          (`/var/lib/d2b/audit/broker-<utc-date>.jsonl`) to
          retain. Files older than this are deleted on every
          day-boundary rotation by the broker (best-effort; failures
          to remove are logged but do not break the audit-write path).
          Set to `0` to disable pruning entirely (unbounded retention).

          **Reserved.** The broker accepts
          `--audit-retention-days <N>` and the runtime prune-on-rotate
          loop is shipping in `packages/d2b-broker/src/audit.rs`,
          but the NixOS module does not yet spawn the broker
          (`d2bd` does so at runtime in a future wiring, and
          this option's value will then thread through
          `daemon-config.json` → `d2bd` → `d2b-broker
          host --audit-retention-days <value>`). Until that wiring
          lands, overriding this option is a no-op at runtime - the
          broker defaults to 14 days regardless.

          The option is exposed now so consumer NixOS configs can
          declare their intended retention ahead of the wiring;
          the runtime path will pick the value up without a config
          break.
        '';
      };
    };

    activation = {
      failClosedOnLegacyGid = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          When true, the d2bGroupMigration helper performs a
          post-migration scan of `/var/lib/d2b` and `/run/d2b`
          and fails activation if any file still has a legacy lifecycle-group numeric gid. Off by default;
          operators flip this to true after confirming clean migration
          on their host.
        '';
      };
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

    waylandDisplay = lib.mkOption {
      type = lib.types.strMatching "^wayland-[0-9]+$";
      default = "wayland-0";
      example = "wayland-1";
      description = ''
        Basename of the host primary compositor's Wayland socket under
        `/run/user/<waylandUser-uid>/`. The GPU sidecar opens this
        socket (`--wayland-sock /run/user/<uid>/<waylandDisplay>`),
        the minijail profile bind-mounts it into the sidecar mount
        namespace, and the privileged broker grants the sidecar uid an
        ACL on exactly this host path.

        Defaults to `wayland-0`, correct when the host compositor is
        the first Wayland server on the seat. Set this to the actual
        socket name when it is not - for example **niri** commonly
        lands on `wayland-1`. A mismatch makes the GPU sidecar fail
        with `vhost-user connection closed` (the socket it was told to
        open does not exist) and the broker refuse the ACL
        (`refusing setfacl on …/wayland-0: expected Socket`), so the
        graphics VM cannot start.

        Find the live value with `echo "$WAYLAND_DISPLAY"` inside the
        host compositor session, or
        `ls /run/user/<uid>/ | grep '^wayland-'`.

        Note: this is a static per-host value. A future enhancement
        (tracked in TODO.md) will source the display from the
        operator's environment at `d2b vm start` time so a single
        host can serve operators on different compositor sockets.
      '';
    };

    audio.inputTargetNode = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "alsa_input.pci-0000_00_1f.3.analog-stereo";
      description = ''
        Optional PipeWire node.name to force d2b VM microphone
        streams to when `d2b.mic = "on"`. Leave at `null` to let
        WirePlumber's normal default-source policy choose the host
        input. Set this on hosts whose default-source metadata does not
        auto-link capture clients reliably.
      '';
    };

    launcherUsers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = ''
        Users to grant launcher-specific lifecycle actions. Launcher
        users are also added to the `d2b` lifecycle group; configured
        Admin users receive that group membership as well.

        When `d2b.daemonExperimental.enable = true`, the same user
        list is also added to the canonical `d2b`
        socket ACL group.

        The framework does NOT create the users - declare them in
        your top-level NixOS config with `users.users.<name> = { …
        };`. d2b only adds the lifecycle group to their
        `extraGroups`.

        Empty list = nobody is a launcher principal. The framework
        still works (sudo + polkit-password prompts cover everything
        the launcher group's allowlist grants).
      '';
    };

    adminUsers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = ''
        Users allowed to request admin-gated daemon operations. This
        covers privileged read-only operations such as `d2b audit`
        AND admin-gated, potentially destructive lifecycle operations
        such as `d2b vm exec` (which opens
        an authenticated command/console session inside a guest).
        Admin users are also added to the `d2b` lifecycle group so
        they can connect over the daemon public socket. Membership
        does not replace the daemon's Admin authorization check.
      '';
    };

    keysDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/d2b/keys";
      example = "/var/lib/d2b/keys";
      description = ''
        Directory where the framework stores managed SSH host keys.
        Mode 0700 is owned by root, with narrow ACLs for authorized
        lifecycle clients.

        Default tracks `${"$"}{stateDir}/keys`. If you override
        `stateDir`, override this too - the option default is a
        literal path because Nix evaluates option defaults
        independently of other options.

        **ADVISORY ONLY in v0.1.0** (same caveat as `stateDir`).
        The directory is independent of Zone resource names; the
        daemon resolves the requested Guest identity.
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
        Extra SSH public keys to authorize for d2b-managed Guests.
        Entries may be either paths to a
        `.pub` file or literal pubkey strings.

        These are merged with the framework's managed key when the
        Guest activation Provider populates the authorized-keys
        file. Empty list means only the managed key is authorized.

        Eval fails if any entry doesn't look like a supported pubkey
        type (ed25519, RSA, ECDSA, security-key variants) or contains
        a `-----BEGIN ... PRIVATE KEY-----` marker.
      '';
    };

    yubikey.enable = lib.mkOption {
      type = lib.types.bool;
      # Host-side USBIP support remains enabled by default.
      default = true;
      example = false;
      description = ''
        Install host-side Yubikey support: the udev rules for vendor
        ID 1050 (so hidraw / raw-USB nodes carry `GROUP="kvm"
        MODE="0660" uaccess`). When at least one enabled VM sets
        `usbip.yubikey = true`, this also loads the host's
        `usbip-host` kernel module so `d2b usb <vm>` can re-bind
        the device into a guest via USBIP.

        Set to `false` on hosts that do not use Yubikeys. With this
        option off the framework does not load `usbip-host` or emit
        Yubico udev rules. The `/dev/kvm` udev rule remains because
        it is not Yubikey-specific.
      '';
    };

    extraSpecialArgs = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = { };
      example = lib.literalExpression ''
        # Pass consumer-specific arguments to guest evaluators. Mirrors
        # home-manager's `extraSpecialArgs` pattern.
        { inherit inputs; }
      '';
      description = ''
        Extra module arguments merged into consumer-supplied guest
        evaluations after the framework's own baseline. Consumer
        keys take precedence on collision.

        Mirrors `home-manager.extraSpecialArgs` from the
        Home-Manager NixOS module - same semantics, same intent.
      '';
    };
  };

  # Top-level option: CIDRs of the host's own physical LAN(s). The
  # Network Provider merges these into each Zone's forwarding policy.
  #
  # Defaults to the empty list; override to your actual subnet.
  # `ip route` on the host will tell you what to put here, e.g.
  # `192.168.1.0/24` for a typical external network with the host at
  # `192.168.1.42/24`.
  options.d2b.hostLanCidrs = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [ ];
    example = [ "192.168.1.0/24" "10.0.0.0/24" ];
    description = ''
      Guests cannot reach the host's neighbours or bypass the
      broker-owned Network effect path. Zone peer traffic remains
      isolated unless the Network resource explicitly opts into
      east-west access.
    '';
  };
}
