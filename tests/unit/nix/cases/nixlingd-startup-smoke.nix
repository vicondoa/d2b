# nix-unit cases migrated from tests/nixlingd-startup-smoke.sh Phase 1.
#
# Eval-only daemon/broker startup surface: systemd tmpfiles rules,
# nixling-priv-broker.{socket,service}, nixlingd.service, and the small
# evidence-record shape assertion the retired shell gate carried before its
# opt-in NL_LIVE section.
{ mkEval, lib, ... }:

let
  canonicalBrokerCaps = [
    "CAP_CHOWN"
    "CAP_DAC_OVERRIDE"
    "CAP_DAC_READ_SEARCH"
    "CAP_FOWNER"
    "CAP_FSETID"
    "CAP_IPC_LOCK"
    "CAP_KILL"
    "CAP_LEASE"
    "CAP_MKNOD"
    "CAP_NET_ADMIN"
    "CAP_NET_RAW"
    "CAP_SETFCAP"
    "CAP_SETGID"
    "CAP_SETPCAP"
    "CAP_SETUID"
    "CAP_SYS_ADMIN"
    "CAP_SYS_RESOURCE"
  ];

  fixture = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    nixling.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    nixling.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ fixture ]).config;
  tmpfiles = cfg.systemd.tmpfiles.rules;
  services = cfg.systemd.services;
  sockets = cfg.systemd.sockets;
  brokerService = services.nixling-priv-broker;
  daemonService = services.nixlingd;
  brokerSocket = sockets.nixling-priv-broker;

  rulesForPath = path:
    builtins.filter (lib.hasInfix (" " + path + " ")) tmpfiles;

  evidenceRecord = {
    wave = "p0";
    timestamp = "2024-01-01T00:00:00Z";
    operatorSignature = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
  };
in
{
  "nixlingd-startup-smoke/tmpfiles-run-nixling" = {
    expr = {
      rootOwnedStickyParent = builtins.elem "d /run/nixling 1770 root nixling -" tmpfiles;
      rootOwnedStickyParentReset = builtins.elem "z /run/nixling 1770 root nixling -" tmpfiles;
      launcherTraverseAcl = builtins.elem "a+ /run/nixling - - - - g::r-x" tmpfiles;
      daemonWriteAcl = builtins.elem "a+ /run/nixling - - - - u:nixlingd:rwx" tmpfiles;
      writeCapableMask = builtins.elem "a+ /run/nixling - - - - m::rwx" tmpfiles;
    };
    expected = {
      rootOwnedStickyParent = true;
      rootOwnedStickyParentReset = true;
      launcherTraverseAcl = true;
      daemonWriteAcl = true;
      writeCapableMask = true;
    };
  };

  "nixlingd-startup-smoke/tmpfiles-usbip-lock-root" = {
    expr = rulesForPath "/run/nixling/locks/usbip";
    expected = [ "d /run/nixling/locks/usbip 0750 root nixlingd -" ];
  };

  "nixlingd-startup-smoke/tmpfiles-audit" = {
    expr = rulesForPath "/var/lib/nixling/audit";
    expected = [ "d /var/lib/nixling/audit 0750 root nixlingd -" ];
  };

  "nixlingd-startup-smoke/tmpfiles-current-bundle" = {
    expr = rulesForPath "/var/lib/nixling/current-bundle";
    expected = [ "d /var/lib/nixling/current-bundle 0755 root root -" ];
  };

  "nixlingd-startup-smoke/tmpfiles-daemon-state" = {
    expr = rulesForPath "/var/lib/nixling/daemon-state";
    expected = [ "d /var/lib/nixling/daemon-state 0700 nixlingd nixlingd -" ];
  };

  "nixlingd-startup-smoke/socket-listen-seqpacket" = {
    expr = brokerSocket.socketConfig.ListenSequentialPacket;
    expected = "/run/nixling/priv.sock";
  };

  "nixlingd-startup-smoke/socket-user" = {
    expr = brokerSocket.socketConfig.SocketUser;
    expected = "root";
  };

  "nixlingd-startup-smoke/socket-group" = {
    expr = brokerSocket.socketConfig.SocketGroup;
    expected = "nixlingd";
  };

  "nixlingd-startup-smoke/socket-mode" = {
    expr = brokerSocket.socketConfig.SocketMode;
    expected = "0660";
  };

  "nixlingd-startup-smoke/socket-fdname" = {
    expr = brokerSocket.socketConfig.FileDescriptorName;
    expected = "priv.sock";
  };

  "nixlingd-startup-smoke/broker-type" = {
    expr = brokerService.serviceConfig.Type;
    expected = "notify";
  };

  "nixlingd-startup-smoke/broker-user" = {
    expr = brokerService.serviceConfig.User;
    expected = "root";
  };

  "nixlingd-startup-smoke/broker-after-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (brokerService.after or [ ]);
    expected = true;
  };

  "nixlingd-startup-smoke/broker-requires-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (brokerService.requires or [ ]);
    expected = true;
  };

  "nixlingd-startup-smoke/broker-group" = {
    expr = brokerService.serviceConfig.Group;
    expected = "nixlingd";
  };

  "nixlingd-startup-smoke/broker-caps-exact-canonical-set" = {
    expr = lib.sort builtins.lessThan brokerService.serviceConfig.CapabilityBoundingSet;
    expected = canonicalBrokerCaps;
  };

  "nixlingd-startup-smoke/daemon-restart-if-changed" = {
    expr = daemonService.restartIfChanged or null;
    expected = true;
  };

  "nixlingd-startup-smoke/daemon-user" = {
    expr = daemonService.serviceConfig.User;
    expected = "nixlingd";
  };

  "nixlingd-startup-smoke/daemon-restrict-address-families" = {
    expr = daemonService.serviceConfig.RestrictAddressFamilies;
    expected = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
  };

  "nixlingd-startup-smoke/daemon-type-notify" = {
    expr = daemonService.serviceConfig.Type;
    expected = "notify";
  };

  "nixlingd-startup-smoke/daemon-notify-access-main" = {
    expr = daemonService.serviceConfig.NotifyAccess;
    expected = "main";
  };

  "nixlingd-startup-smoke/daemon-timeout-start" = {
    expr = daemonService.serviceConfig.TimeoutStartSec;
    expected = "5min";
  };

  "nixlingd-startup-smoke/daemon-killmode-process" = {
    expr = daemonService.serviceConfig.KillMode;
    expected = "process";
  };

  "nixlingd-startup-smoke/daemon-execstop-hook" = {
    expr = lib.hasInfix "nixling-host-shutdown-hook" daemonService.serviceConfig.ExecStop;
    expected = true;
  };

  "nixlingd-startup-smoke/daemon-wants-broker-socket" = {
    expr = builtins.elem "nixling-priv-broker.socket" (daemonService.wants or [ ]);
    expected = true;
  };

  "nixlingd-startup-smoke/daemon-after-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (daemonService.after or [ ]);
    expected = true;
  };

  "nixlingd-startup-smoke/daemon-wants-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (daemonService.wants or [ ]);
    expected = true;
  };

  "nixlingd-startup-smoke/evidence-record-shape" = {
    expr =
      evidenceRecord.wave == "p0"
      && evidenceRecord.timestamp != ""
      && evidenceRecord.operatorSignature != "";
    expected = true;
  };
}
