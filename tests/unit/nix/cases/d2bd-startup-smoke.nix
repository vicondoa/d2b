# nix-unit cases migrated from tests/d2bd-startup-smoke.sh Phase 1.
#
# Eval-only daemon/broker startup surface: systemd tmpfiles rules,
# d2b-priv-broker.{socket,service}, d2bd.service, and the small
# evidence-record shape assertion the retired shell gate carried before its
# opt-in D2B_LIVE section.
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
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ fixture ]).config;
  tmpfiles = cfg.systemd.tmpfiles.rules;
  services = cfg.systemd.services;
  sockets = cfg.systemd.sockets;
  brokerService = services.d2b-priv-broker;
  daemonService = services.d2bd;
  brokerSocket = sockets.d2b-priv-broker;

  rulesForPath = path:
    builtins.filter (lib.hasInfix (" " + path + " ")) tmpfiles;

  evidenceRecord = {
    wave = "p0";
    timestamp = "2024-01-01T00:00:00Z";
    operatorSignature = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
  };
in
{
  "d2bd-startup-smoke/tmpfiles-run-d2b" = {
    expr = {
      rootOwnedStickyParent = builtins.elem "d /run/d2b 1770 root d2b -" tmpfiles;
      rootOwnedStickyParentReset = builtins.elem "z /run/d2b 1770 root d2b -" tmpfiles;
      launcherTraverseAcl = builtins.elem "a+ /run/d2b - - - - g::r-x" tmpfiles;
      daemonWriteAcl = builtins.elem "a+ /run/d2b - - - - u:d2bd:rwx" tmpfiles;
      userTraverseAcl = builtins.elem "a+ /run/d2b - - - - u:alice:--x" tmpfiles;
      writeCapableMask = builtins.elem "a+ /run/d2b - - - - m::rwx" tmpfiles;
      noDefaultAclMask =
        !(builtins.elem "a+ /run/d2b - - - - default:m::rwx" tmpfiles);
      finalRunD2bRules = lib.take 2 (lib.reverseList
        (builtins.filter (rule: lib.hasPrefix "a+ /run/d2b - - - - " rule) tmpfiles));
    };
    expected = {
      rootOwnedStickyParent = true;
      rootOwnedStickyParentReset = true;
      launcherTraverseAcl = true;
      daemonWriteAcl = true;
      userTraverseAcl = true;
      writeCapableMask = true;
      noDefaultAclMask = true;
      finalRunD2bRules = [
        "a+ /run/d2b - - - - m::rwx"
        "a+ /run/d2b - - - - u:alice:--x"
      ];
    };
  };

  "d2bd-startup-smoke/tmpfiles-usbip-lock-root" = {
    expr = rulesForPath "/run/d2b/locks/usbip";
    expected = [ "d /run/d2b/locks/usbip 0750 root d2bd -" ];
  };

  "d2bd-startup-smoke/tmpfiles-audit" = {
    expr = rulesForPath "/var/lib/d2b/audit";
    expected = [ "d /var/lib/d2b/audit 0750 root d2bd -" ];
  };

  "d2bd-startup-smoke/tmpfiles-current-bundle" = {
    expr = rulesForPath "/var/lib/d2b/current-bundle";
    expected = [ "d /var/lib/d2b/current-bundle 0755 root root -" ];
  };

  "d2bd-startup-smoke/tmpfiles-daemon-state" = {
    expr = rulesForPath "/var/lib/d2b/daemon-state";
    expected = [ "d /var/lib/d2b/daemon-state 0700 d2bd d2bd -" ];
  };

  "d2bd-startup-smoke/socket-listen-seqpacket" = {
    expr = brokerSocket.socketConfig.ListenSequentialPacket;
    expected = "/run/d2b/priv.sock";
  };

  "d2bd-startup-smoke/socket-user" = {
    expr = brokerSocket.socketConfig.SocketUser;
    expected = "root";
  };

  "d2bd-startup-smoke/socket-group" = {
    expr = brokerSocket.socketConfig.SocketGroup;
    expected = "d2bd";
  };

  "d2bd-startup-smoke/socket-mode" = {
    expr = brokerSocket.socketConfig.SocketMode;
    expected = "0660";
  };

  "d2bd-startup-smoke/socket-fdname" = {
    expr = brokerSocket.socketConfig.FileDescriptorName;
    expected = "priv.sock";
  };

  "d2bd-startup-smoke/broker-type" = {
    expr = brokerService.serviceConfig.Type;
    expected = "notify";
  };

  "d2bd-startup-smoke/broker-user" = {
    expr = brokerService.serviceConfig.User;
    expected = "root";
  };

  "d2bd-startup-smoke/broker-after-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (brokerService.after or [ ]);
    expected = true;
  };

  "d2bd-startup-smoke/broker-requires-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (brokerService.requires or [ ]);
    expected = true;
  };

  "d2bd-startup-smoke/broker-group" = {
    expr = brokerService.serviceConfig.Group;
    expected = "d2bd";
  };

  "d2bd-startup-smoke/broker-caps-exact-canonical-set" = {
    expr = lib.sort builtins.lessThan brokerService.serviceConfig.CapabilityBoundingSet;
    expected = canonicalBrokerCaps;
  };

  "d2bd-startup-smoke/daemon-restart-if-changed" = {
    expr = daemonService.restartIfChanged or null;
    expected = true;
  };

  "d2bd-startup-smoke/daemon-user" = {
    expr = daemonService.serviceConfig.User;
    expected = "d2bd";
  };

  "d2bd-startup-smoke/daemon-prestart-does-not-repair-run-d2b-acl" =
    let
      prestart = daemonService.serviceConfig.ExecStartPre or [];
    in {
    expr = {
      chmod = builtins.any (cmd: lib.hasInfix "chmod" cmd && lib.hasInfix "/run/d2b" cmd) prestart;
      acl = builtins.any (cmd: lib.hasInfix "setfacl" cmd && lib.hasInfix "/run/d2b" cmd) prestart;
    };
    expected = {
      chmod = false;
      acl = false;
    };
  };

  "d2bd-startup-smoke/daemon-does-not-skip-kernel-module-check" = {
    expr = daemonService.environment.D2B_SKIP_KERNEL_MODULE_CHECK or null;
    expected = null;
  };

  "d2bd-startup-smoke/daemon-restrict-address-families" = {
    expr = daemonService.serviceConfig.RestrictAddressFamilies;
    expected = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
  };

  "d2bd-startup-smoke/daemon-type-notify" = {
    expr = daemonService.serviceConfig.Type;
    expected = "notify";
  };

  "d2bd-startup-smoke/daemon-notify-access-main" = {
    expr = daemonService.serviceConfig.NotifyAccess;
    expected = "main";
  };

  "d2bd-startup-smoke/daemon-timeout-start" = {
    expr = daemonService.serviceConfig.TimeoutStartSec;
    expected = "5min";
  };

  "d2bd-startup-smoke/daemon-killmode-process" = {
    expr = daemonService.serviceConfig.KillMode;
    expected = "process";
  };

  "d2bd-startup-smoke/daemon-execstop-hook" = {
    expr = lib.hasInfix "d2b-host-shutdown-hook" daemonService.serviceConfig.ExecStop;
    expected = true;
  };

  "d2bd-startup-smoke/daemon-wants-broker-socket" = {
    expr = builtins.elem "d2b-priv-broker.socket" (daemonService.wants or [ ]);
    expected = true;
  };

  "d2bd-startup-smoke/daemon-after-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (daemonService.after or [ ]);
    expected = true;
  };

  "d2bd-startup-smoke/daemon-wants-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service" (daemonService.wants or [ ]);
    expected = true;
  };

  "d2bd-startup-smoke/evidence-record-shape" = {
    expr =
      evidenceRecord.wave == "p0"
      && evidenceRecord.timestamp != ""
      && evidenceRecord.operatorSignature != "";
    expected = true;
  };
}
