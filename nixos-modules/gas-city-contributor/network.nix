{ config, lib, pkgs, ... }:

let
  cfg = config.services.gasCityContributor;
  buildBuddyEnabled =
    cfg.buildBuddy.enable || cfg.credentials.buildBuddyApiKeyFile != null;
  package = cfg.package;
  python = "${package}/bin/python3";
  activation = "${package}/share/gas-city-contributor/pack/scripts/service-activation.py";
  egressDirectory = "/run/gascity-egress";
  egressSocket = "${egressDirectory}/egress.sock";
  relayAuth = builtins.hashString "sha256"
    "gascity-fdproxy:${cfg.repository.githubSlug}:${cfg.repository.rigName}";
  requiredIntegrationDomains = [
    "api.github.com"
    "discord.com"
    "gateway.discord.gg"
    "github.com"
  ];
  domainArgs = lib.concatMapStringsSep " " (
    domain: "--allowed-domain ${lib.escapeShellArg domain}"
  ) (lib.unique (cfg.network.allowedDomains ++ requiredIntegrationDomains)
    ++ lib.optional cfg.check.enable "cache.nixos.org"
    ++ lib.optional buildBuddyEnabled "remote.buildbuddy.io");
  egressUids =
    [
      config.users.users.gascity.uid
      config.users.users.gascity-agent.uid
      config.users.users.gascity-discord.uid
      config.users.users.gascity-publisher.uid
    ]
    ++ lib.optional cfg.check.enable config.users.users.gascity-check.uid
    ++ lib.optional buildBuddyEnabled config.users.users.gascity-buildbuddy-proxy.uid;
  uidArgs = lib.concatMapStringsSep " " (uid: "--allowed-uid ${toString uid}") egressUids;
  egressStart = "${python} ${activation} egress-peer"
    + " --socket ${lib.escapeShellArg egressSocket}"
    + " --socket-group gascity-egress-channel"
    + " --auth-token-env GC_FDPROXY_AUTH"
    + " --allowed-port 443"
    + " ${uidArgs} ${domainArgs}";
in
{
  config = lib.mkIf cfg.enable {
    # This is deliberately an additional table.  It does not assign the
    # host's base filter policy and therefore does not replace unrelated
    # consumer firewall rules.
    networking.nftables.enable = true;
    networking.nftables.ruleset = lib.mkAfter ''
      table inet gascity_contributor {
        chain output {
          type filter hook output priority 20; policy accept;

          # The two host-loopback control ports are service-owned. Socket
          # owner metadata is available on locally generated output packets.
          oifname "lo" tcp dport ${toString cfg.ports.supervisor} meta skuid != ${toString config.users.users.gascity.uid} drop
          oifname "lo" tcp dport ${toString cfg.ports.dolt} meta skuid != ${toString config.users.users.gascity.uid} drop

          # The egress peer never opens private, link-local, loopback,
          # multicast, or metadata destinations even if a caller supplies a
          # name resolving to one of them.
          meta skuid ${toString config.users.users.gascity-egress.uid} ip daddr 127.0.0.53 udp dport 53 accept
          meta skuid ${toString config.users.users.gascity-egress.uid} ip daddr 127.0.0.53 tcp dport 53 accept
          meta skuid ${toString config.users.users.gascity-egress.uid} ip6 daddr ::1 udp dport 53 accept
          meta skuid ${toString config.users.users.gascity-egress.uid} ip6 daddr ::1 tcp dport 53 accept
          meta skuid ${toString config.users.users.gascity-egress.uid} ip daddr {
            0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8,
            169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24,
            192.0.2.0/24, 192.168.0.0/16, 198.18.0.0/15,
            198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4,
            240.0.0.0/4
          } drop
          meta skuid ${toString config.users.users.gascity-egress.uid} ip6 daddr {
            ::/128, ::1/128, fc00::/7, fe80::/10, ff00::/8,
            2001:db8::/32
          } drop
          meta skuid ${toString config.users.users.gascity-egress.uid} tcp dport != 443 drop
          meta skuid ${toString config.users.users.gascity-egress.uid} udp dport != 53 drop
          meta skuid ${toString config.users.users.gascity-egress.uid} ip protocol != { tcp, udp } drop
          meta skuid ${toString config.users.users.gascity-egress.uid} ip6 nexthdr != { tcp, udp } drop
        }
      }
    '';

    systemd.services.gascity-egress = {
      description = "Gas City allowlisting egress sidecar";
      before = [ "gas-city-contributor.service" ];
      requires = [ "gascity-free-space-monitor.service" ];
      bindsTo = [ "gascity-free-space-monitor.service" ];
      after = [ "gascity-free-space-monitor.service" ];
      unitConfig = {
        PartOf = "gas-city-contributor.service";
        StartLimitIntervalSec = 60;
        StartLimitBurst = 5;
      };
      serviceConfig = {
        Type = "exec";
        Slice = "gascity-contributor.slice";
        User = "gascity-egress";
        Group = "gascity-egress-channel";
        SupplementaryGroups = [ "gascity-contributor" "gascity-egress" ];
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectProc = "invisible";
        ProcSubset = "pid";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        RestrictSUIDSGID = true;
        RestrictRealtime = true;
        LockPersonality = true;
        UMask = "0077";
        KillMode = "control-group";
        RuntimeDirectory = "gascity-egress";
        RuntimeDirectoryMode = "0750";
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        ReadWritePaths = [ egressDirectory ];
        InaccessiblePaths = [
          "-/etc/shadow"
          "-/etc/gshadow"
          "-/run/systemd"
          "-/nix/var/nix/daemon-socket/socket"
          "-/proc/kcore"
          "-/proc/keys"
        ];
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
          "~@mount"
          "~@raw-io"
          "chown"
        ];
        Environment = [
          "GC_FDPROXY_AUTH=${relayAuth}"
          "GC_FDPROXY_SOCKET=${egressSocket}"
          "SSL_CERT_FILE=${package}/etc/ssl/certs/ca-bundle.crt"
        ];
        ExecStart = egressStart;
        Restart = "on-failure";
        RestartSec = "2s";
      };
    };
  };
}
