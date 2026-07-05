# Guest-side baseline applied to every d2b VM.
#
# Layered in by `host.nix`'s `microvm.vms` translation. Each entry
# here uses `lib.mkDefault` so a per-VM module can override.
#
# Component-specific concerns (graphics, TPM, USBIP, Entra-ID) live
# in their own files under `nixos-modules/components/` and are NOT
# imported here.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  usersGroupsJsonPath =
    let
      matches = builtins.match ".*update-users-groups\\.pl ([^[:space:]\n]+).*" config.system.activationScripts.users.text;
    in
    if matches == null then null else builtins.head matches;
  usersGroupsJson = builtins.toJSON {
    inherit (config.users) mutableUsers;
    groups = lib.mapAttrsToList (name: group: {
      inherit name;
      gid = group.gid or null;
      members = group.members or [ ];
    }) config.users.groups;
    users = lib.mapAttrsToList (name: user: {
      inherit name;
      autoSubUidGidRange = user.autoSubUidGidRange or false;
      createHome = user.createHome or false;
      description = user.description or "";
      expires = user.expires or null;
      group = user.group or "nogroup";
      hashedPassword = user.hashedPassword or null;
      hashedPasswordFile = user.hashedPasswordFile or null;
      home = user.home or "/var/empty";
      homeMode = user.homeMode or "700";
      initialHashedPassword = user.initialHashedPassword or null;
      initialPassword = user.initialPassword or null;
      isSystemUser = user.isSystemUser or false;
      password = user.password or null;
      shell = user.shell or null;
      subGidRanges = user.subGidRanges or [ ];
      subUidRanges = user.subUidRanges or [ ];
      uid = user.uid or null;
    }) config.users.users;
  };
in
{
  options.d2b.sshUser = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = ''
      SSH user to populate with the framework-managed pubkey + the
      operator-supplied userAuthorizedKeys at guest boot. Populated
      by host.nix from the host-side `d2b.vms.<name>.ssh.user`
      option; for net VMs it stays null (host.nix instead points the
      service at `root`).
    '';
  };

  options.d2b.sudo = lib.mkEnableOption ''
    passwordless sudo for the VM's SSH user. When enabled, a
    NOPASSWD sudoers rule is added for the SSH user.
  '';

  config = {
  # Passwordless sudo for the SSH user when d2b.sudo is enabled.
  security.sudo.extraRules = lib.mkIf (cfg.sudo && cfg.sshUser != null) [{
    users = [ cfg.sshUser ];
    commands = [{ command = "ALL"; options = [ "NOPASSWD" ]; }];
  }];

  # Every d2b VM uses systemd-networkd. The host runs a DHCP
  # server on each env's LAN bridge; per-VM static-IP overrides come
  # from the dnsmasq host-reservation set up by network.nix.
  networking.useNetworkd = lib.mkDefault true;
  systemd.network.enable = lib.mkDefault true;
  services.resolved.enable = lib.mkDefault true;

  # IPv6 is off by default for d2b VMs (networking
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

  # d2b guests boot from a minimal root overlay where /etc may not exist yet.
  # Create it before NixOS' user/group activation for normal switch paths.
  system.activationScripts.d2bEnsureEtcForUsers = ''
    mkdir -p /etc
    chmod 0755 /etc
  '';
  system.activationScripts.users.deps = lib.mkBefore [ "d2bEnsureEtcForUsers" ];

  # On d2b microVM cold boots the activation script runs during initrd before
  # switch-root, so the standard users snippet can write passwd/group data into
  # the transient initrd root. Re-run the idempotent generated users snippet
  # early after switch-root and before socket/basic units resolve users/groups.
  systemd.services.d2b-refresh-users-after-switch-root = {
    description = "Refresh declarative users/groups after switch-root";
    wantedBy = [ "sysinit.target" ];
    after = [ "local-fs.target" ];
    before = [ "sysinit.target" "sockets.target" "basic.target" ];
    path = [
      pkgs.coreutils
      pkgs.findutils
      pkgs.getent
      pkgs.glibc.bin
      pkgs.gnugrep
      pkgs.shadow
      pkgs.util-linux
    ];
    unitConfig.DefaultDependencies = false;
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      StandardOutput = "journal+console";
      StandardError = "journal+console";
      ExecStart = pkgs.writeShellScript "d2b-refresh-users-after-switch-root" ''
        set -euxo pipefail
        exec > >(tee /dev/console) 2>&1
        echo "d2b-refresh-users: start"
        pwd
        ls -ld / /etc /var /var/lib || true
        mkdir -p /etc
        chmod 0755 /etc
        mkdir -p /var/lib/nixos
        for map in /var/lib/nixos/uid-map /var/lib/nixos/gid-map /var/lib/nixos/auto-subuid-map; do
          if [ -e "$map" ] && ! head -c 1 "$map" | grep -q '{'; then
            mv "$map" "$map.corrupt.$(date +%s)"
          fi
        done
        printf '%s\n' ${lib.escapeShellArg usersGroupsJson} > /run/d2b-users-groups.json
        wc -c /run/d2b-users-groups.json
        od -An -tx1 -N32 /run/d2b-users-groups.json
        echo "d2b-refresh-users: before generated users snippet"
        ${builtins.replaceStrings
          (lib.optional (usersGroupsJsonPath != null) usersGroupsJsonPath)
          (lib.optional (usersGroupsJsonPath != null) "/run/d2b-users-groups.json")
          config.system.activationScripts.users.text}
        echo "d2b-refresh-users: after generated users snippet"
        ls -l /etc/passwd /etc/group /etc/shadow || true
        test -s /etc/passwd
        test -s /etc/group
        echo "d2b-refresh-users: complete"
      '';
    };
  };

  # ---------------------------------------------------------------------------
  # Per-VM nix store: load the db.dump from the host-injected
  # /run/d2b-store-meta share into the guest's local Nix DB
  # (/nix/var/nix/db/). Without this, `nix-store --query --valid` and
  # `nix-shell` both reject any closure path they didn't register
  # themselves — so writableStoreOverlay-based Home Manager + ad-hoc
  # `nix-shell -p hello` would fail. The host writes db.dump as the
  # `registration` output of `pkgs.closureInfo`, which is the format
  # `nix-store --load-db` consumes.
  #
  # Fires on every boot AND whenever the host publishes steady-state
  # store metadata by bumping `current` (via the path-trigger below).
  # Live `d2b switch <vm>` additionally runs through authenticated
  # guest-control activation before the broker commits the host-side
  # current pointers.
  # ---------------------------------------------------------------------------
  systemd.services.d2b-load-store-db = {
    description = "Load d2b per-VM closure into the guest's local nix DB";
    wantedBy = [ "multi-user.target" ];
    after = [ "nix-daemon.socket" ];
    unitConfig = {
      ConditionPathExists = "/run/d2b-store-meta/current/db.dump";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = false;
      ExecStart = "${pkgs.writeShellScript "d2b-load-store-db" ''
        set -euo pipefail
        DUMP=/run/d2b-store-meta/current/db.dump
        if [ ! -f "$DUMP" ]; then
          echo "d2b-load-store-db: $DUMP missing — skipping." >&2
          exit 0
        fi
        # The dump is trusted (came from the host's hardlinked closure
        # of this VM's own toplevel), so --no-check-sigs is appropriate.
        ${pkgs.nix}/bin/nix-store --load-db < "$DUMP"
        echo "d2b-load-store-db: loaded $(wc -l < /run/d2b-store-meta/current/store-paths) paths from $DUMP"
      ''}";
    };
  };

  # Path trigger: re-run d2b-load-store-db whenever `current`
  # changes (host fired `d2b-<vm>-store-sync` → bumped current).
  # PathChanged fires on rename/replace of the symlink target.
  systemd.paths.d2b-load-store-db = {
    description = "Watch d2b store-meta/current for closure updates";
    wantedBy = [ "multi-user.target" ];
    pathConfig = {
      PathChanged = "/run/d2b-store-meta/current";
    };
  };

  # ---------------------------------------------------------------------------
  # Inject the framework-managed host pubkey + userAuthorizedKeys
  # into ~ssh-user/.ssh/authorized_keys (or root's, for net VMs that
  # don't declare an ssh.user).
  #
  # The host-keys/ share (/run/d2b-host-keys/) is staged by the
  # host's d2bGenerateKeys activation script and refreshed on
  # every host switch. We dedupe at write time so adding the same
  # operator pubkey under both d2b.site.userAuthorizedKeys and
  # d2b.vms.<vm>.userAuthorizedKeys doesn't bloat the file.
  # ---------------------------------------------------------------------------
  systemd.services.d2b-load-host-keys = {
    description = "Inject d2b-managed pubkey + user-authorized-keys for ssh-user";
    wantedBy = [ "multi-user.target" ];
    after = [ "local-fs.target" ];
    unitConfig = {
      ConditionPathIsMountPoint = "/run/d2b-host-keys";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${pkgs.writeShellScript "d2b-load-host-keys" ''
        set -euo pipefail
        SHARE=/run/d2b-host-keys
        # Resolve target user. Empty = root (net VM case).
        SSH_USER="${if cfg.sshUser == null then "root" else cfg.sshUser}"

        # Look up the user's home from /etc/passwd at boot — declarative
        # users.users.<u>.home would also work but requires propagating
        # the value through extra options.
        if ! USER_HOME=$(${pkgs.glibc.getent}/bin/getent passwd "$SSH_USER" | ${pkgs.coreutils}/bin/cut -d: -f6); then
          echo "d2b-load-host-keys: user '$SSH_USER' not found in /etc/passwd — skipping" >&2
          exit 0
        fi
        if [ -z "$USER_HOME" ]; then
          echo "d2b-load-host-keys: user '$SSH_USER' has empty home — skipping" >&2
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
          echo "d2b-load-host-keys: could not resolve primary group for '$SSH_USER' (gid=$SSH_GID) — skipping" >&2
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

        echo "d2b-load-host-keys: $(${pkgs.coreutils}/bin/wc -l < "$AUTH_KEYS") key(s) installed in $AUTH_KEYS for $SSH_USER"
      ''}";
    };
  };
  };
}
