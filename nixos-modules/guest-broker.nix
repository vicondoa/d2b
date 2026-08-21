# d2b-broker guest profile
#
# Every workload Guest gets a separate socket-activated broker instance. The
# executable is the same host-tool derivation used by the Host profile; only
# the process-start profile and authority roots differ.
{ config, lib, pkgs, name, d2bHostTools, d2bUsePrebuiltHostTools ? false, ... }:

let
  prebuilt =
    if d2bUsePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  brokerSourcePackage = d2bHostTools.broker;
  brokerPackage =
    if prebuilt ? "selectPackage"
    then prebuilt.selectPackage "d2b-broker" brokerSourcePackage
    else brokerSourcePackage;
  brokerSocket = "/run/d2b/guest-broker.sock";
  brokerState = "/var/lib/d2b/guest-broker";
  brokerAudit = "/var/lib/d2b/guest-audit";
  brokerBundle = "/etc/d2b/guest-bundle.json";
  brokerUid = config.users.users.d2bd.uid or 997;
  brokerGid = config.users.groups.d2bd.gid or 997;
in
{
  users.groups.d2bd = {
    gid = lib.mkDefault 997;
  };
  users.users.d2bd = {
    isSystemUser = true;
    group = "d2bd";
    uid = lib.mkDefault 997;
  };

  environment.systemPackages = [ brokerPackage ];

  systemd.tmpfiles.rules = [
    "d /run/d2b 0750 root d2bd -"
    "d ${brokerState} 0700 root d2bd -"
    "d ${brokerAudit} 0750 root d2bd -"
  ];

  systemd.sockets.d2b-broker-guest = {
    description = "d2b Guest privileged broker socket";
    wantedBy = [ "sockets.target" ];
    requires = [ "systemd-tmpfiles-setup.service" ];
    after = [ "systemd-tmpfiles-setup.service" ];
    socketConfig = {
      ListenSequentialPacket = brokerSocket;
      SocketUser = "root";
      SocketGroup = "d2bd";
      SocketMode = "0660";
      Accept = false;
      FileDescriptorName = "priv.sock";
    };
  };

  systemd.services.d2b-broker-guest = {
    description = "d2b Guest privileged broker";
    requires = [
      "d2b-broker-guest.socket"
      "systemd-tmpfiles-setup.service"
    ];
    after = [
      "d2b-broker-guest.socket"
      "systemd-tmpfiles-setup.service"
      "local-fs.target"
    ];
    environment = {
      RUST_LOG = lib.mkDefault "info";
    };
    serviceConfig = {
      Type = "notify";
      NotifyAccess = "main";
      User = "root";
      Group = "d2bd";
      CapabilityBoundingSet = [
        "CAP_DAC_OVERRIDE"
        "CAP_DAC_READ_SEARCH"
        "CAP_FOWNER"
        "CAP_KILL"
        "CAP_SETGID"
        "CAP_SETUID"
        "CAP_SYS_ADMIN"
        "CAP_SYS_RESOURCE"
      ];
      AmbientCapabilities = [ "" ];
      NoNewPrivileges = false;
      KillMode = "process";
      PrivateTmp = true;
      ProtectHome = true;
      ProtectClock = true;
      ProtectProc = "invisible";
      RestrictAddressFamilies = [ "AF_UNIX" "AF_VSOCK" ];
      SystemCallArchitectures = "native";
      UMask = "0027";
      ExecStart =
        "${brokerPackage}/bin/d2b-broker guest " +
        "--authority-id guest-${name} " +
        "--audit-dir ${brokerAudit} " +
        "--bundle-path ${brokerBundle} " +
        "--state-dir ${brokerState} " +
        "--d2bd-uid ${toString brokerUid} " +
        "--d2bd-gid ${toString brokerGid}";
      Restart = "on-failure";
      RestartSec = "2s";
      StandardOutput = "journal";
      StandardError = "journal";
      SyslogIdentifier = "d2b-broker-guest";
    };
  };
}
