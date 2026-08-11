{ config, lib, pkgs, ... }:

let
  cfg = config.services.gasCityContributor;
  buildBuddyEnabled =
    cfg.buildBuddy.enable || cfg.credentials.buildBuddyApiKeyFile != null;
  package = cfg.package;
  python = "${package}/bin/python3";
  activation = "${package}/share/gas-city-contributor/pack/scripts/service-activation.py";
  runtimeRoot = "/run/gascity-contributor";
  egressSocket = "${runtimeRoot}/egress.sock";
  relayAuth = builtins.hashString "sha256"
    "gascity-fdproxy:${cfg.repository.githubSlug}:${cfg.repository.rigName}";
  domainArgs = lib.concatMapStringsSep " " (
    domain: "--allowed-domain ${lib.escapeShellArg domain}"
  ) (cfg.network.allowedDomains
    ++ lib.optional cfg.check.enable "cache.nixos.org"
    ++ lib.optional buildBuddyEnabled "remote.buildbuddy.io");
  egressUids =
    [ 45101 ]
    ++ lib.optional cfg.check.enable 45105
    ++ lib.optional buildBuddyEnabled 45106;
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
          oifname "lo" tcp dport ${toString cfg.ports.supervisor} meta skuid != 45100 drop
          oifname "lo" tcp dport ${toString cfg.ports.dolt} meta skuid != 45100 drop

          # The egress peer never opens private, link-local, loopback,
          # multicast, or metadata destinations even if a caller supplies a
          # name resolving to one of them.
          meta skuid 45104 ip daddr 127.0.0.53 udp dport 53 accept
          meta skuid 45104 ip daddr 127.0.0.53 tcp dport 53 accept
          meta skuid 45104 ip6 daddr ::1 udp dport 53 accept
          meta skuid 45104 ip6 daddr ::1 tcp dport 53 accept
          meta skuid 45104 ip daddr {
            0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8,
            169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24,
            192.0.2.0/24, 192.168.0.0/16, 198.18.0.0/15,
            198.51.100.0/24, 203.0.113.0/24, 224.0.0.0/4,
            240.0.0.0/4
          } drop
          meta skuid 45104 ip6 daddr {
            ::/128, ::1/128, fc00::/7, fe80::/10, ff00::/8,
            2001:db8::/32
          } drop
          meta skuid 45104 tcp dport != 443 drop
          meta skuid 45104 udp dport != 53 drop
          meta skuid 45104 ip protocol != { tcp, udp } drop
          meta skuid 45104 ip6 nexthdr != { tcp, udp } drop
        }
      }
    '';

    systemd.services.gascity-egress = {
      description = "Gas City allowlisting egress sidecar";
      before = [ "gas-city-contributor.service" ];
      unitConfig = {
        PartOf = "gas-city-contributor.service";
        StartLimitIntervalSec = 60;
        StartLimitBurst = 5;
      };
      serviceConfig = {
        Type = "exec";
        Slice = "gascity-contributor.slice";
        User = "gascity-egress";
        Group = "gascity-egress";
        SupplementaryGroups = [ "gascity-contributor" "gascity-egress-channel" ];
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
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        ReadWritePaths = [ runtimeRoot ];
        InaccessiblePaths = [
          "/etc/shadow"
          "/etc/gshadow"
          "/run/systemd"
          "/nix/var/nix/daemon-socket/socket"
          "/proc/kcore"
          "/proc/keys"
        ];
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
          "~@mount"
          "~@raw-io"
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
