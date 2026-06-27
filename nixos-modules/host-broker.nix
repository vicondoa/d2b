# d2b-priv-broker.{socket,service}
#
# Socket-activated privileged broker per ADR 0001 (`unsafe_code =
# "deny"` quarantine boundary). Daemon connects to /run/d2b/priv.sock
# via the systemd-owned listening socket; the broker binary inherits
# the listen fd via SD_LISTEN_FDS and never binds the path itself
# (eliminates address-in-use races and lets systemd own the socket's
# ACL contract).
#
# Authority: framework maintainer.
# The bounding set is exactly the listed caps (no CAP_SYS_PTRACE, no
# CAP_CHOWN outside the cgroup-delegation startup window).
{ inputs }:

{ pkgs, lib, config, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };

  # filter out `target/` dev caches from the source
  # so the Nix copy stays small (broker target alone is ~6 GB).
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;

  brokerSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-priv-broker";
    version =
      (builtins.fromTOML (builtins.readFile ../packages/d2b-priv-broker/Cargo.toml))
        .package.version;
    src = packagesSrc;
    sourceRoot = "source/d2b-priv-broker";
    cargoLock = {
      lockFile = ../packages/d2b-priv-broker/Cargo.lock;
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
    meta.description = "d2b privileged broker (uid 0 host-mutation surface)";
  };
  brokerPackage = if prebuilt ? "d2b-priv-broker" then prebuilt."d2b-priv-broker" else brokerSourcePackage;

  bundleManifestPath =
    cfg.site.bundle.currentManifest or "/etc/d2b/bundle.json";

  auditRetentionDays = cfg.site.audit.retentionDays or 14;
in

{
  # the broker NixOS module was previously gated behind
  # `cfg.daemonExperimental.enable`. v1.1 makes the broker
  # socket/service default-on (ADR 0015 daemon-only clean break), so
  # this broker module is no longer gated by the toggle. The toggle
  # itself is NOT a no-op: it defaults `true` and still functionally
  # gates the daemon control plane (`d2bd`, daemon-config, and the
  # bundle-artifact group ownership) in `nixos-modules/host-daemon.nix`
  # and the `*-json.nix` emitters — setting it `false` reverts the host
  # to the unsupported pre-daemon legacy state. It is no longer
  # evidence-auto-flipped; `nixos-modules/assertions.nix` deliberately
  # does not warn on it (the option default makes `isDefined` true even
  # when consumers do not set it).
  # The module body always materializes the broker — there is no
  # `mkIf` wrapper. The legacy gating semantics are documented in
  # `docs/how-to/migrate-nixos-to-daemon.md` § Recovery.
  config = {

    environment.systemPackages = [ brokerPackage ];

    # broker-owned state + bundle dirs; /run/d2b itself is owned by
    # root:d2b 1770 with explicit ACLs from host-daemon.nix (canonical;
    # this module MUST NOT touch /run/d2b tmpfiles to avoid the
    # runtime-dir ownership conflict).
    systemd.tmpfiles.rules = [
      "d /var/lib/d2b/audit 0750 root d2bd -"
      "d /var/lib/d2b/current-bundle 0755 root root -"
    ];

    # Declare d2b.slice as a top-level slice (systemd naming
    # convention: no dashes in the basename = top-level). The broker's
    # DEFAULT_DELEGATED_PARENT_SLICE constant was updated to
    # /sys/fs/cgroup/d2b.slice to match (was previously
    # /sys/fs/cgroup/system.slice/d2b.slice, but that nested
    # form requires systemd-style naming `system-d2b.slice`
    # which would put it at system.slice/system-d2b.slice
    # NOT system.slice/d2b.slice). Top-level is simpler and
    # matches the broker's cgroup walk-up logic.
    systemd.slices.d2b = {
      description = "Slice for d2b-managed VMs and broker-spawned runners";
      sliceConfig = {
        Delegate = "cpu memory pids io cpuset";
      };
    };

    # SOCKET-ACTIVATED. systemd owns the bind, ACL, and lifecycle
    # of /run/d2b/priv.sock. The broker process receives the fd
    # via SD_LISTEN_FDS=1 LISTEN_FDS=1 LISTEN_FDNAMES=priv.sock and
    # MUST NOT bind/listen itself when activated this way.
    systemd.sockets.d2b-priv-broker = {
      description = "d2b privileged broker socket";
      wantedBy = [ "sockets.target" ];
      requires = [ "systemd-tmpfiles-setup.service" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      socketConfig = {
        ListenSequentialPacket = "/run/d2b/priv.sock";
        SocketUser = "root";
        SocketGroup = "d2bd";
        SocketMode = "0660";
        Accept = false;
        FileDescriptorName = "priv.sock";
      };
    };

    systemd.services.d2b-priv-broker = {
      description = "d2b privileged broker (uid 0 host-mutation surface)";
      documentation = [
        "https://github.com/vicondoa/d2b/blob/main/docs/adr/0001-broker-privilege-quarantine.md"
        "https://github.com/vicondoa/d2b/blob/main/docs/reference/privileges.md"
      ];
      # Socket-activated; service activation comes from the socket unit.
      # No wantedBy here.
      requires = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
      ];
      after = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
        "local-fs.target"
      ];
      # Keep the broker at INFO by default so high-volume DEBUG payload
      # diagnostics do not flood journal/OTel exports. Operators can
      # temporarily force RUST_LOG=debug from host configuration when
      # investigating a broker live-handler failure.
      environment = {
        RUST_LOG = lib.mkDefault "info";
        # Point at host tools (NixOS has no /usr/sbin/nft default).
        D2B_BROKER_NFT_BINARY = "${pkgs.nftables}/bin/nft";
        # iproute2 binary lives in /bin not /sbin on NixOS.
        D2B_BROKER_IP_BINARY = "${pkgs.iproute2}/bin/ip";
        # usbip binary from linuxPackages_latest.usbip.
        D2B_BROKER_USBIP_BINARY = "${pkgs.linuxPackages_latest.usbip}/bin/usbip";
      };

      # ApplyNftables / SpawnRunner mount-prep ops invoke nft /
      # setfacl / mount via PATH lookup. Add the
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
        # in packages/d2b-priv-broker/src/runtime.rs after the
        # SD_LISTEN_FDS adoption + cgroup delegation sequence.
        #
        Type = "notify";
        NotifyAccess = "main";

        # Broker MUST be uid 0 for cgroup v2 delegation + tap/bridge
        # ops + nft mutations.
        User = "root";
        # Group=d2bd matches priv.sock peer-cred group.
        Group = "d2bd";

        # Canonical CapabilityBoundingSet. The set includes every cap
        # the broker may need to pass through to
        # a spawned runner. Child role caps live in the bundle's
        # role profile; if the broker's bounding set is narrower
        # than the role's cap list, capset(2) in the child fails
        # with EPERM and the child exits silently with
        # CHILD_EXIT_CAPSET. The full set required by virtiofsd /
        # cloud-hypervisor / swtpm / gpu role profiles is
        # CAP_NET_ADMIN / CAP_NET_RAW / CAP_DAC_OVERRIDE /
        # CAP_DAC_READ_SEARCH / CAP_SYS_ADMIN / CAP_SETUID /
        # CAP_SETGID / CAP_FOWNER / CAP_SETPCAP / CAP_CHOWN /
        # CAP_FSETID / CAP_MKNOD / CAP_SETFCAP / CAP_SYS_RESOURCE /
        # CAP_IPC_LOCK / CAP_LEASE. CAP_LEASE is required for DiskInit
        # to prove no other process has a declared disk image open before
        # fd-bound posture repair or mkfs. CAP_KILL is required for the audited
        # SignalRunner broker op: root inside this bounding set still
        # gets EPERM from pidfd_send_signal(2) without it when signaling
        # runner UIDs outside the broker's own credential set.
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
          "CAP_LEASE"
          "CAP_KILL"
        ];
        AmbientCapabilities = [ "" ];
        # NoNewPrivileges=false because the broker re-execs after the
        # cgroup-delegation startup window with a reduced cap set.
        NoNewPrivileges = false;

        # Place broker under d2b.slice so the broker's cgroup path
        # matches the broker's DEFAULT_DELEGATED_PARENT_SLICE.
        Slice = "d2b.slice";
        Delegate = true;
        # Broker-spawned runners are placed into dedicated d2b.slice
        # role leaves and handed to d2bd by pidfd. Stopping/restarting
        # the socket-activated broker must not make systemd recursively
        # tear down those runner leaves; targeted teardown is broker/daemon
        # mediated via pidfd/CgroupKill instead.
        KillMode = "process";

        # Isolation knobs compatible with broker's job.
        PrivateTmp = true;
        # ProtectHome=true also tmpfs-masks /run/user/<uid> which the
        # audio role needs to reach the
        # Wayland user's PipeWire socket. Drop it — the broker
        # has no business reading /home regardless, and CAP_DAC_*
        # in the bounding set is gated by minijail profile per
        # spawned role anyway.
        ProtectHome = false;
        ProtectClock = true;
        ProtectProc = "invisible";
        # ProcSubset=pid blocks the broker from reading
        # /proc/sys/kernel/random/uuid which audit.rs
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

        # Resolve d2bd uid/gid at start time. Broker validates
        # SO_PEERCRED on incoming connections against these.
        ExecStartPre = "+${pkgs.writeShellScript "d2b-priv-broker-prep" ''
          set -euo pipefail
          uid=$(${pkgs.coreutils}/bin/id -u d2bd)
          gid=$(${pkgs.coreutils}/bin/id -g d2bd)
          ${pkgs.systemd}/bin/systemctl set-environment D2BD_UID="$uid"
          ${pkgs.systemd}/bin/systemctl set-environment D2BD_GID="$gid"
        ''}";

        # NOTE: NO --socket-path here. With SD_LISTEN_FDS the broker
        # MUST adopt the inherited fd; the --socket-path flag is the
        # non-activated-mode fallback only.
        ExecStart =
          "${brokerPackage}/bin/d2b-priv-broker serve " +
          "--audit-dir /var/lib/d2b/audit " +
          "--audit-retention-days ${toString auditRetentionDays} " +
          "--bundle-path ${bundleManifestPath} " +
          "--state-dir ${cfg.site.stateDir}";

        Restart = "on-failure";
        RestartSec = "2s";

        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "d2b-priv-broker";
      };
    };

    # Daemon: Wants= (not Requires=) the broker socket so daemon
    # serving doesn't hard-fail if the socket-activated broker has
    # idled out. Daemon code reconnects on ENOENT/ECONNRESET per
    # request.
    #
    # NOTE: The previous guard `lib.mkIf (config.systemd.services ?
    # d2bd)` caused infinite recursion in the NixOS module system
    # because it forced evaluation of `systemd.services` from within a
    # definition contributing to `systemd.services`. This broker module
    # is unconditional (no `mkIf` wrapper); only host-daemon.nix is
    # gated on `daemonExperimental.enable`. The guard is unnecessary: we
    # unconditionally merge the wants/after entries here — they are
    # harmless if the `d2bd` unit is absent (e.g. when
    # `daemonExperimental.enable = false` drops the daemon config),
    # since systemd merges these at the unit-file level.
    systemd.services.d2bd = {
      wants = [ "d2b-priv-broker.socket" ];
      after = [ "d2b-priv-broker.socket" ];
    };
  };
}
