# P0: nixling-priv-broker.{socket,service}
#
# Socket-activated privileged broker per ADR 0001 (`unsafe_code =
# "deny"` quarantine boundary). Daemon connects to /run/nixling/priv.sock
# via the systemd-owned listening socket; the broker binary inherits
# the listen fd via SD_LISTEN_FDS and never binds the path itself
# (eliminates address-in-use races and lets systemd own the socket's
# ACL contract).
#
# Authority: paydro@nixos 2026-05-30 plan-review R4 panel 9/9 signoff.
# Per plan.md §"Canonical broker CapabilityBoundingSet" the bounding
# set is exactly the listed caps (no CAP_SYS_PTRACE, no CAP_CHOWN
# outside the cgroup-delegation startup window).
{ inputs }:

{ pkgs, lib, config, ... }:

let
  cfg = config.nixling;

  brokerPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-priv-broker";
    version =
      (builtins.fromTOML (builtins.readFile ../packages/nixling-priv-broker/Cargo.toml))
        .package.version;
    src = lib.cleanSource ../packages/nixling-priv-broker;
    cargoLock = {
      lockFile = ../packages/nixling-priv-broker/Cargo.lock;
    };
    cargoBuildFlags = [ "--no-default-features" ];
    doCheck = false;
    meta.description = "nixling privileged broker (uid 0 host-mutation surface)";
  };

  bundleManifestPath =
    cfg.site.bundle.currentManifest or "/etc/nixling/bundle.json";

  auditRetentionDays = cfg.site.audit.retentionDays or 14;
in

{
  config = lib.mkIf cfg.daemonExperimental.enable {

    environment.systemPackages = [ brokerPackage ];

    # broker-owned state + bundle dirs; /run/nixling itself is owned by
    # nixlingd:nixling-launchers 0750 from host-daemon.nix (canonical;
    # this module MUST NOT touch /run/nixling tmpfiles to avoid the
    # ph0-runtime-dir-canonicalize conflict).
    systemd.tmpfiles.rules = [
      "d /var/lib/nixling/audit 0750 root nixlingd -"
      "d /var/lib/nixling/current-bundle 0755 root root -"
    ];

    # SOCKET-ACTIVATED. systemd owns the bind, ACL, and lifecycle
    # of /run/nixling/priv.sock. The broker process receives the fd
    # via SD_LISTEN_FDS=1 LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock and
    # MUST NOT bind/listen itself when activated this way.
    systemd.sockets.nixling-priv-broker = {
      description = "nixling privileged broker socket";
      wantedBy = [ "sockets.target" ];
      socketConfig = {
        ListenSequentialPacket = "/run/nixling/priv.sock";
        SocketUser = "root";
        SocketGroup = "nixlingd";
        SocketMode = "0660";
        Accept = false;
        FileDescriptorName = "priv.sock";
      };
    };

    systemd.services.nixling-priv-broker = {
      description = "nixling privileged broker (uid 0 host-mutation surface)";
      documentation = [
        "https://github.com/vicondoa/nixling/blob/main/docs/adr/0001-broker-privilege-quarantine.md"
        "https://github.com/vicondoa/nixling/blob/main/docs/reference/privileges.md"
      ];
      # Socket-activated; service activation comes from the socket unit.
      # No wantedBy here.
      requires = [ "nixling-priv-broker.socket" ];
      after = [ "nixling-priv-broker.socket" "local-fs.target" ];

      serviceConfig = {
        # Type=notify so the daemon can deterministically observe
        # READY=1 after the broker has adopted the listen fd and
        # completed cgroup delegation. Pair with sd_notify(READY=1)
        # in packages/nixling-priv-broker/src/runtime.rs after the
        # SD_LISTEN_FDS adoption + cgroup delegation sequence.
        # (ph0-cgroup-delegation-sequence covers the latter.)
        Type = "notify";
        NotifyAccess = "main";

        # Broker MUST be uid 0 for cgroup v2 delegation + tap/bridge
        # ops + nft mutations.
        User = "root";
        # Group=nixlingd matches priv.sock peer-cred group.
        Group = "nixlingd";

        # Canonical CapabilityBoundingSet per plan §"Canonical broker
        # CapabilityBoundingSet". NO CAP_SYS_PTRACE.
        CapabilityBoundingSet = [
          "CAP_NET_ADMIN"
          "CAP_NET_RAW"
          "CAP_DAC_OVERRIDE"
          "CAP_DAC_READ_SEARCH"
          "CAP_SYS_ADMIN"
          "CAP_SETUID"
          "CAP_SETGID"
          "CAP_FOWNER"
        ];
        AmbientCapabilities = [ "" ];
        # NoNewPrivileges=false because the broker re-execs after the
        # cgroup-delegation startup window with a reduced cap set.
        NoNewPrivileges = false;

        # Isolation knobs compatible with broker's job.
        PrivateTmp = true;
        ProtectHome = true;
        ProtectClock = true;
        ProtectProc = "invisible";
        ProcSubset = "pid";
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_NETLINK"
          "AF_VSOCK"
          "AF_INET"
          "AF_INET6"
        ];
        SystemCallArchitectures = "native";
        UMask = "0027";

        # Resolve nixlingd uid/gid at start time. Broker validates
        # SO_PEERCRED on incoming connections against these.
        ExecStartPre = "+${pkgs.writeShellScript "nixling-priv-broker-prep" ''
          set -euo pipefail
          uid=$(${pkgs.coreutils}/bin/id -u nixlingd)
          gid=$(${pkgs.coreutils}/bin/id -g nixlingd)
          ${pkgs.systemd}/bin/systemctl set-environment NIXLINGD_UID="$uid"
          ${pkgs.systemd}/bin/systemctl set-environment NIXLINGD_GID="$gid"
        ''}";

        # NOTE: NO --socket-path here. With SD_LISTEN_FDS the broker
        # MUST adopt the inherited fd; the --socket-path flag is the
        # non-activated-mode fallback only.
        ExecStart =
          "${brokerPackage}/bin/nixling-priv-broker serve " +
          "--audit-dir /var/lib/nixling/audit " +
          "--audit-retention-days ${toString auditRetentionDays} " +
          "--bundle-path ${bundleManifestPath}";

        Restart = "on-failure";
        RestartSec = "2s";

        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "nixling-priv-broker";
      };
    };

    # Daemon: Wants= (not Requires=) the broker socket so daemon
    # serving doesn't hard-fail if the socket-activated broker has
    # idled out. Daemon code reconnects on ENOENT/ECONNRESET per
    # request (ph0-broker-socket-activation contract).
    #
    # NOTE: The previous guard `lib.mkIf (config.systemd.services ?
    # nixlingd)` caused infinite recursion in the NixOS module system
    # because it forced evaluation of `systemd.services` from within a
    # definition contributing to `systemd.services`. Since both this
    # module and host-daemon.nix are gated on daemonExperimental.enable,
    # the guard is redundant: we unconditionally merge the
    # wants/after entries here (they no-op if host-daemon.nix is absent
    # since systemd merges at unit-file level).
    systemd.services.nixlingd = {
      wants = [ "nixling-priv-broker.socket" ];
      after = [ "nixling-priv-broker.socket" ];
    };
  };
}
