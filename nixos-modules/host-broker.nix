{ inputs }:

{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  localRoot = cfg._realmPrincipals.localRoot;
  controllerPrincipal = localRoot.controller;
  brokerPrincipal = localRoot.broker;
  brokerSocketPrincipal = localRoot.socketPrincipals.broker;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  brokerSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-priv-broker";
    version =
      (builtins.fromTOML (builtins.readFile ../packages/Cargo.toml))
        .workspace.package.version;
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" =
        "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [
      "--package"
      "d2b-priv-broker"
      "--no-default-features"
    ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
      [build]
      rustc-wrapper = ""
      EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    meta.description = "d2b local-root privileged broker";
  };
  brokerPackage =
    if builtins.hasAttr "d2b-priv-broker" prebuilt
    then prebuilt."d2b-priv-broker"
    else brokerSourcePackage;
  bundleManifestPath =
    cfg.site.bundle.currentManifest or "/etc/d2b/bundle.json";
  auditRetentionDays = cfg.site.audit.retentionDays or 14;
in
{
  config = {
    environment.systemPackages = [ brokerPackage ];

    systemd.tmpfiles.rules = [
      "d /run/d2b/broker 0750 ${brokerPrincipal} ${controllerPrincipal} -"
      "d /var/lib/d2b/audit 0750 ${brokerPrincipal} ${controllerPrincipal} -"
      "d /var/lib/d2b/current-bundle 0755 root root -"
    ];

    systemd.slices.d2b = {
      description = "Slice for d2b-managed processes";
      sliceConfig.Delegate = "cpu memory pids io cpuset";
    };

    systemd.sockets.d2b-priv-broker = {
      description = "d2b local-root privileged broker socket";
      wantedBy = [ "sockets.target" ];
      requires = [ "systemd-tmpfiles-setup.service" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      socketConfig = {
        ListenSequentialPacket = "/run/d2b/broker.sock";
        SocketUser = brokerSocketPrincipal.owner;
        SocketGroup = brokerSocketPrincipal.group;
        SocketMode = brokerSocketPrincipal.mode;
        Accept = false;
        FileDescriptorName = "priv.sock";
        Service = "d2b-priv-broker.service";
        RemoveOnStop = true;
      };
    };

    systemd.services.d2b-priv-broker = {
      description = "d2b local-root privileged broker";
      documentation = [
        "https://github.com/vicondoa/d2b/blob/main/docs/adr/0002-non-root-daemon-and-privileged-broker.md"
        "https://github.com/vicondoa/d2b/blob/main/docs/reference/privileges.md"
      ];
      requires = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
      ];
      after = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
        "local-fs.target"
      ];
      environment = {
        RUST_LOG = lib.mkDefault "info";
        D2B_BROKER_NFT_BINARY = "${pkgs.nftables}/bin/nft";
        D2B_BROKER_IP_BINARY = "${pkgs.iproute2}/bin/ip";
        D2B_BROKER_USBIP_BINARY =
          "${pkgs.linuxPackages_latest.usbip}/bin/usbip";
      };
      path = with pkgs; [
        nftables
        acl
        iproute2
        util-linux
      ];
      serviceConfig = {
        Type = "notify";
        NotifyAccess = "main";
        User = brokerPrincipal;
        Group = brokerPrincipal;
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
        NoNewPrivileges = false;
        Slice = "d2b.slice";
        Delegate = true;
        KillMode = "process";
        PrivateTmp = true;
        ProtectHome = false;
        ProtectClock = true;
        ProtectProc = "invisible";
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_NETLINK"
          "AF_VSOCK"
          "AF_INET"
          "AF_INET6"
        ];
        SystemCallArchitectures = "native";
        UMask = "0027";
        EnvironmentFile = "-/run/d2b/broker/priv-broker.env";
        ExecStartPre = "+${pkgs.writeShellScript "d2b-priv-broker-prep" ''
          set -euo pipefail
          env_file=/run/d2b/broker/priv-broker.env
          env_tmp=/run/d2b/broker/priv-broker.env.new
          uid=$(${pkgs.coreutils}/bin/id -u ${controllerPrincipal})
          gid=$(${pkgs.coreutils}/bin/id -g ${controllerPrincipal})
          umask 0077
          {
            printf 'D2BD_UID=%s\n' "$uid"
            printf 'D2BD_GID=%s\n' "$gid"
          } > "$env_tmp"
          ${pkgs.coreutils}/bin/chown \
            ${brokerPrincipal}:${controllerPrincipal} "$env_tmp"
          ${pkgs.coreutils}/bin/chmod 0640 "$env_tmp"
          ${pkgs.coreutils}/bin/mv -f "$env_tmp" "$env_file"
        ''}";
        ExecStart =
          "${brokerPackage}/bin/d2b-priv-broker serve "
          + "--audit-dir /var/lib/d2b/audit "
          + "--audit-retention-days ${toString auditRetentionDays} "
          + "--bundle-path ${bundleManifestPath} "
          + "--realm-controllers-path /etc/d2b/realm-controllers.json "
          + "--realm-identity-path /etc/d2b/realm-identity.json "
          + "--state-dir ${cfg.site.stateDir}";
        Restart = "on-failure";
        RestartSec = "2s";
        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "d2b-priv-broker";
      };
    };
  };
}
