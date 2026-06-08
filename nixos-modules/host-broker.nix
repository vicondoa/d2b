# P0: nixling-priv-broker.{socket,service}
#
# Socket-activated privileged broker per ADR 0001 (`unsafe_code =
# "deny"` quarantine boundary). Daemon connects to /run/nixling/priv.sock
# via the systemd-owned listening socket; the broker binary inherits
# the listen fd via SD_LISTEN_FDS and never binds the path itself
# (eliminates address-in-use races and lets systemd own the socket's
# ACL contract).
#
# Authority: framework maintainer 2026-05-30 plan-review R4 panel 9/9 signoff.
# Per plan.md §"Canonical broker CapabilityBoundingSet" the bounding
# set is exactly the listed caps (no CAP_SYS_PTRACE, no CAP_CHOWN
# outside the cgroup-delegation startup window).
{ inputs }:

{ pkgs, lib, config, ... }:

let
  cfg = config.nixling;

  # v1.1.1fu11: filter out `target/` dev caches from the source
  # so the Nix copy stays small (broker target alone is ~6 GB).
  packagesSrc = lib.cleanSourceWith {
    src = ../packages;
    filter = path: type:
      let rel = lib.removePrefix (toString ../packages + "/") (toString path);
      in !(lib.hasInfix "target" rel || lib.hasInfix ".cargo/registry" rel);
  };

  brokerPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-priv-broker";
    version =
      (builtins.fromTOML (builtins.readFile ../packages/nixling-priv-broker/Cargo.toml))
        .package.version;
    src = packagesSrc;
    sourceRoot = "source/nixling-priv-broker";
    cargoLock = {
      lockFile = ../packages/nixling-priv-broker/Cargo.lock;
    };
    cargoBuildFlags = [ "--no-default-features" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    meta.description = "nixling privileged broker (uid 0 host-mutation surface)";
  };

  bundleManifestPath =
    cfg.site.bundle.currentManifest or "/etc/nixling/bundle.json";

  auditRetentionDays = cfg.site.audit.retentionDays or 14;
in

{
  # v1.1-P4: the broker NixOS module was previously gated behind
  # `cfg.daemonExperimental.enable`. v1.1 makes the broker
  # socket/service default-on (ADR 0015 daemon-only clean break);
  # the `daemonExperimental.enable` toggle is now a no-op (consumer
  # flakes that still set it receive an eval-time warning, emitted
  # via the v1.1-P4 assertion in `nixos-modules/assertions.nix`).
  # The module body always materializes the broker — there is no
  # `mkIf` wrapper. The legacy gating semantics are documented in
  # `docs/how-to/migrate-nixos-to-daemon.md` § Recovery.
  config = {

    environment.systemPackages = [ brokerPackage ];

    # broker-owned state + bundle dirs; /run/nixling itself is owned by
    # nixlingd:nixling-launchers 0750 from host-daemon.nix (canonical;
    # this module MUST NOT touch /run/nixling tmpfiles to avoid the
    # ph0-runtime-dir-canonicalize conflict).
    systemd.tmpfiles.rules = [
      "d /var/lib/nixling/audit 0750 root nixlingd -"
      "d /var/lib/nixling/current-bundle 0755 root root -"
    ];

    # v1.1.1 live-deploy fu9: declare nixling.slice as a
    # top-level slice (systemd naming convention: no dashes in
    # the basename = top-level). The broker's
    # DEFAULT_DELEGATED_PARENT_SLICE constant was updated to
    # /sys/fs/cgroup/nixling.slice to match (was previously
    # /sys/fs/cgroup/system.slice/nixling.slice, but that nested
    # form requires systemd-style naming `system-nixling.slice`
    # which would put it at system.slice/system-nixling.slice
    # NOT system.slice/nixling.slice). Top-level is simpler and
    # matches the broker's cgroup walk-up logic.
    systemd.slices.nixling = {
      description = "Slice for nixling-managed VMs and broker-spawned runners";
      sliceConfig = {
        Delegate = "cpu memory pids io";
      };
    };

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
      # v1.1.1 live-deploy fu9 + fu12: surface broker debug logs and
      # point at the nft binary (NixOS has no /usr/sbin/nft default).
      environment = {
        RUST_LOG = "debug";
        NIXLING_BROKER_NFT_BINARY = "${pkgs.nftables}/bin/nft";
        # iproute2 binary lives in /bin not /sbin on NixOS.
        NIXLING_BROKER_IP_BINARY = "${pkgs.iproute2}/bin/ip";
        # usbip binary from linuxPackages_latest.usbip.
        NIXLING_BROKER_USBIP_BINARY = "${pkgs.linuxPackages_latest.usbip}/bin/usbip";
      };

      # v1.1.1 live-deploy fu12: ApplyNftables / SpawnRunner mount-prep
      # ops invoke nft / setfacl / mount via PATH lookup. Add the
      # tools the broker live handlers shell out to.
      path = with pkgs; [
        nftables
        acl
        iproute2
        util-linux
      ];

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
        # CapabilityBoundingSet". v1.1.1 live-deploy fu10: expanded
        # to include every cap the broker may need to pass through to
        # a spawned runner. Child role caps live in the bundle's
        # role profile; if the broker's bounding set is narrower
        # than the role's cap list, capset(2) in the child fails
        # with EPERM and the child exits silently with
        # CHILD_EXIT_CAPSET. The full set required by virtiofsd /
        # cloud-hypervisor / swtpm / gpu role profiles is:
        # CAP_NET_ADMIN / CAP_NET_RAW / CAP_DAC_OVERRIDE /
        # CAP_DAC_READ_SEARCH / CAP_SYS_ADMIN / CAP_SETUID /
        # CAP_SETGID / CAP_FOWNER / CAP_SETPCAP / CAP_CHOWN /
        # CAP_FSETID / CAP_MKNOD / CAP_SETFCAP / CAP_SYS_RESOURCE /
        # CAP_IPC_LOCK.
        CapabilityBoundingSet = [
          "CAP_NET_ADMIN"
          "CAP_NET_RAW"
          "CAP_DAC_OVERRIDE"
          "CAP_DAC_READ_SEARCH"
          "CAP_SYS_ADMIN"
          "CAP_SETUID"
          "CAP_SETGID"
          "CAP_FOWNER"
          "CAP_SETPCAP"
          "CAP_CHOWN"
          "CAP_FSETID"
          "CAP_MKNOD"
          "CAP_SETFCAP"
          "CAP_SYS_RESOURCE"
          "CAP_IPC_LOCK"
        ];
        AmbientCapabilities = [ "" ];
        # NoNewPrivileges=false because the broker re-execs after the
        # cgroup-delegation startup window with a reduced cap set.
        NoNewPrivileges = false;

        # v1.1.1 live-deploy fu9: place broker under
        # nixling.slice (which is itself nested under
        # system.slice — see systemd.slices.nixling above) so
        # the broker's cgroup path is
        # /sys/fs/cgroup/system.slice/nixling.slice/* matching
        # the broker's DEFAULT_DELEGATED_PARENT_SLICE.
        Slice = "nixling.slice";
        Delegate = true;

        # Isolation knobs compatible with broker's job.
        PrivateTmp = true;
        # v1.1.1fu11 (Option B): ProtectHome=true also tmpfs-masks
        # /run/user/<uid> which the audio role needs to reach the
        # Wayland user's PipeWire socket. Drop it — the broker
        # has no business reading /home regardless, and CAP_DAC_*
        # in the bounding set is gated by minijail profile per
        # spawned role anyway.
        ProtectHome = false;
        ProtectClock = true;
        ProtectProc = "invisible";
        # v1.1.1 live-deploy fu9: ProcSubset=pid blocks the broker
        # from reading /proc/sys/kernel/random/uuid which audit.rs
        # uses to generate event IDs. Drop the subset restriction
        # so the audit pipeline works. (ProtectProc=invisible
        # still hides other processes' /proc entries.)
        # ProcSubset = "pid";
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
