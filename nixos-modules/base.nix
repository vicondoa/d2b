# Guest-side baseline applied to every nixling VM.
#
# Layered in by `host.nix`'s `microvm.vms` translation. Each entry
# here uses `lib.mkDefault` so a per-VM module can override.
#
# Component-specific concerns (graphics, TPM, USBIP, Entra-ID) live
# in their own files under `nixos-modules/components/` and are NOT
# imported here.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
in
{
  options.nixling.sshUser = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = ''
      SSH user to populate with the framework-managed pubkey + the
      operator-supplied userAuthorizedKeys at guest boot. Populated
      by host.nix from the host-side `nixling.vms.<name>.ssh.user`
      option; for net VMs it stays null (host.nix instead points the
      service at `root`).
    '';
  };

  options.nixling.sudo = lib.mkEnableOption ''
    passwordless sudo for the VM's SSH user. When enabled, a
    NOPASSWD sudoers rule is added for the SSH user.
  '';

  config = {
  # Passwordless sudo for the SSH user when nixling.sudo is enabled.
  security.sudo.extraRules = lib.mkIf (cfg.sudo && cfg.sshUser != null) [{
    users = [ cfg.sshUser ];
    commands = [{ command = "ALL"; options = [ "NOPASSWD" ]; }];
  }];

  # Every nixling VM uses systemd-networkd. The host runs a DHCP
  # server on each env's LAN bridge; per-VM static-IP overrides come
  # from the dnsmasq host-reservation set up by network.nix.
  networking.useNetworkd = lib.mkDefault true;
  systemd.network.enable = lib.mkDefault true;
  services.resolved.enable = lib.mkDefault true;

  # IPv6 is off by default for nixling VMs (networking
  # hardening). The bridge plumbing,
  # nftables rules in net.nix, and dnsmasq are all IPv4-only by
  # construction; auto-configured IPv6 link-local addresses (which
  # systemd-networkd would assign by default) leak unintended
  # multicast / NDP traffic onto the LAN bridge. Disable at the
  # kernel level so even loopback IPv6 is dark. mkDefault so a
  # per-VM module can flip them back if you actually need v6.
  boot.kernel.sysctl = lib.mkDefault {
    "net.ipv6.conf.all.disable_ipv6"     = 1;
    "net.ipv6.conf.default.disable_ipv6" = 1;
  };
  systemd.network.networks."10-eth-dhcp" = lib.mkDefault {
    matchConfig.Type = "ether";
    networkConfig = {
      DHCP = "ipv4";
      LinkLocalAddressing = "no";
      IPv6AcceptRA = false;
    };
  };

  services.openssh = {
    enable = lib.mkDefault true;
    settings = {
      PermitRootLogin = lib.mkDefault "no";
      PasswordAuthentication = lib.mkDefault false;
      # Also disable keyboard-interactive so workload VMs only
      # advertise publickey. Otherwise sshd offers
      # "publickey,keyboard-interactive" and an attacker can attempt
      # PAM-driven prompts.
      KbdInteractiveAuthentication = lib.mkDefault false;
    };
  };

  networking.firewall.enable = lib.mkDefault true;
  networking.firewall.allowedTCPPorts = [ 22 ];

  time.timeZone = lib.mkDefault "America/Los_Angeles";
  i18n.defaultLocale = lib.mkDefault "en_US.UTF-8";

  system.stateVersion = lib.mkDefault "26.05";

  # ---------------------------------------------------------------------------
  # Per-VM nix store: load the db.dump from the host-injected
  # /run/nixling-store-meta share into the guest's local Nix DB
  # (/nix/var/nix/db/). Without this, `nix-store --query --valid` and
  # `nix-shell` both reject any closure path they didn't register
  # themselves — so writableStoreOverlay-based Home Manager + ad-hoc
  # `nix-shell -p hello` would fail. The host writes db.dump as the
  # `registration` output of `pkgs.closureInfo`, which is the format
  # `nix-store --load-db` consumes.
  #
  # Fires on every boot AND whenever the host publishes steady-state
  # store metadata by bumping `current` (via the path-trigger below).
  # Live `nixling switch <vm>` additionally runs through authenticated
  # guest-control activation before the broker commits the host-side
  # current pointers.
  # ---------------------------------------------------------------------------
  systemd.services.nixling-load-store-db = {
    description = "Load nixling per-VM closure into the guest's local nix DB";
    wantedBy = [ "multi-user.target" ];
    after = [ "nix-daemon.socket" ];
    unitConfig = {
      ConditionPathExists = "/run/nixling-store-meta/current/db.dump";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = false;
      ExecStart = "${pkgs.writeShellScript "nl-load-store-db" ''
        set -euo pipefail
        DUMP=/run/nixling-store-meta/current/db.dump
        if [ ! -f "$DUMP" ]; then
          echo "nixling-load-store-db: $DUMP missing — skipping." >&2
          exit 0
        fi
        # The dump is trusted (came from the host's hardlinked closure
        # of this VM's own toplevel), so --no-check-sigs is appropriate.
        ${pkgs.nix}/bin/nix-store --load-db < "$DUMP"
        echo "nixling-load-store-db: loaded $(wc -l < /run/nixling-store-meta/current/store-paths) paths from $DUMP"
      ''}";
    };
  };

  # Path trigger: re-run nixling-load-store-db whenever `current`
  # changes (host fired `nixling-<vm>-store-sync` → bumped current).
  # PathChanged fires on rename/replace of the symlink target.
  systemd.paths.nixling-load-store-db = {
    description = "Watch nixling store-meta/current for closure updates";
    wantedBy = [ "multi-user.target" ];
    pathConfig = {
      PathChanged = "/run/nixling-store-meta/current";
    };
  };

  # ---------------------------------------------------------------------------
  # Inject the framework-managed host pubkey + userAuthorizedKeys
  # into ~ssh-user/.ssh/authorized_keys (or root's, for net VMs that
  # don't declare an ssh.user).
  #
  # The host-keys/ share (/run/nixling-host-keys/) is staged by the
  # host's nixlingGenerateKeys activation script and refreshed on
  # every host switch. We dedupe at write time so adding the same
  # operator pubkey under both nixling.site.userAuthorizedKeys and
  # nixling.vms.<vm>.userAuthorizedKeys doesn't bloat the file.
  # ---------------------------------------------------------------------------
  systemd.services.nixling-load-host-keys = {
    description = "Inject nixling-managed pubkey + user-authorized-keys for ssh-user";
    wantedBy = [ "multi-user.target" ];
    after = [ "local-fs.target" ];
    unitConfig = {
      ConditionPathIsMountPoint = "/run/nixling-host-keys";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${pkgs.writeShellScript "nixling-load-host-keys" ''
        set -euo pipefail
        SHARE=/run/nixling-host-keys
        # Resolve target user. Empty = root (net VM case).
        SSH_USER="${if cfg.sshUser == null then "root" else cfg.sshUser}"

        # Look up the user's home from /etc/passwd at boot — declarative
        # users.users.<u>.home would also work but requires propagating
        # the value through extra options.
        if ! USER_HOME=$(${pkgs.glibc.getent}/bin/getent passwd "$SSH_USER" | ${pkgs.coreutils}/bin/cut -d: -f6); then
          echo "nixling-load-host-keys: user '$SSH_USER' not found in /etc/passwd — skipping" >&2
          exit 0
        fi
        if [ -z "$USER_HOME" ]; then
          echo "nixling-load-host-keys: user '$SSH_USER' has empty home — skipping" >&2
          exit 0
        fi

        # Resolve primary group via the same getent (passwd field 4 is
        # the GID; convert GID → group name with getent group). v0.1.5
        # fix: pre-v0.1.5 the script assumed group = $SSH_USER which
        # only holds when the user was created with `users.users.<u>.group
        # = "<u>"` or via DynamicUser. NixOS's `isNormalUser = true`
        # default puts the user in the `users` group, so the old
        # assumption EACCESed `install: invalid group '<u>'`.
        SSH_GID=$(${pkgs.glibc.getent}/bin/getent passwd "$SSH_USER" | ${pkgs.coreutils}/bin/cut -d: -f4)
        SSH_GROUP=$(${pkgs.glibc.getent}/bin/getent group "$SSH_GID" | ${pkgs.coreutils}/bin/cut -d: -f1)
        if [ -z "$SSH_GROUP" ]; then
          echo "nixling-load-host-keys: could not resolve primary group for '$SSH_USER' (gid=$SSH_GID) — skipping" >&2
          exit 0
        fi

        ${pkgs.coreutils}/bin/install -d -m 0700 -o "$SSH_USER" \
          -g "$SSH_GROUP" "$USER_HOME/.ssh"

        AUTH_KEYS="$USER_HOME/.ssh/authorized_keys"
        TMP=$(${pkgs.coreutils}/bin/mktemp "$USER_HOME/.ssh/.authorized_keys.XXXXXX")
        trap '${pkgs.coreutils}/bin/rm -f "$TMP"' EXIT

        {
          [ -f "$SHARE/host.pub" ] && ${pkgs.coreutils}/bin/cat "$SHARE/host.pub" || true
          [ -f "$SHARE/user-authorized-keys" ] && ${pkgs.coreutils}/bin/cat "$SHARE/user-authorized-keys" || true
        } \
          | ${pkgs.gnused}/bin/sed -E 's/[[:space:]]+$//' \
          | ${pkgs.gnugrep}/bin/grep -E '^(ssh-|ecdsa-|sk-)' \
          | ${pkgs.coreutils}/bin/sort -u \
          > "$TMP" || true

        ${pkgs.coreutils}/bin/install -m 0600 -o "$SSH_USER" \
          -g "$SSH_GROUP" "$TMP" "$AUTH_KEYS"

        echo "nixling-load-host-keys: $(${pkgs.coreutils}/bin/wc -l < "$AUTH_KEYS") key(s) installed in $AUTH_KEYS for $SSH_USER"
      ''}";
    };
  };
  };
}
