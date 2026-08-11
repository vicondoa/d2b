{ config, lib, ... }:

let
  cfg = config.services.gasCityContributor;
  buildBuddyEnabled =
    cfg.buildBuddy.enable || cfg.credentials.buildBuddyApiKeyFile != null;
  sharedGroup = "gascity-contributor";
  allServiceUsers = {
    gascity = {
      uid = 45100;
      description = "Gas City lifecycle owner";
      home = "/var/lib/gascity-contributor/home";
    };
    gascity-agent = {
      uid = 45101;
      description = "Gas City ACP launcher";
      home = "/var/lib/gascity-agent";
    };
    gascity-discord = {
      uid = 45102;
      description = "Gas City Discord integration";
      home = "/var/lib/gascity-discord";
    };
    gascity-publisher = {
      uid = 45103;
      description = "Gas City GitHub publisher";
      home = "/var/lib/gascity-publisher";
    };
    gascity-egress = {
      uid = 45104;
      description = "Gas City allowlisting egress peer";
      home = "/var/lib/gascity-egress";
    };
    gascity-check = {
      uid = 45105;
      description = "Gas City uncredentialed check runner";
      home = "/var/lib/gascity-check";
    };
    gascity-buildbuddy-proxy = {
      uid = 45106;
      description = "Gas City BuildBuddy credential proxy";
      home = "/var/lib/gascity-buildbuddy-proxy";
    };
  };

  mkServiceUser = name: details: {
    isSystemUser = true;
    uid = details.uid;
    group = name;
    extraGroups =
      [ sharedGroup ]
      ++ {
        gascity = [ "gascity-agent-channel" "gascity-discord-channel" "gascity-publisher-channel" ];
        gascity-agent = [ "gascity-agent-channel" "gascity-egress-channel" ];
        gascity-discord = [ "gascity-discord-channel" "gascity-egress-channel" ];
        gascity-publisher = [ "gascity-publisher-channel" "gascity-egress-channel" ];
        gascity-egress = [ "gascity-egress-channel" ];
        gascity-check = [ "gascity-egress-channel" ];
        gascity-buildbuddy-proxy = [ "gascity-egress-channel" ];
      }.${name};
    inherit (details) description home;
    createHome = false;
    shell = "/run/current-system/sw/bin/nologin";
  };

  serviceUsers =
    (lib.removeAttrs allServiceUsers [ "gascity-check" "gascity-buildbuddy-proxy" ])
    // lib.optionalAttrs cfg.check.enable {
      gascity-check = allServiceUsers.gascity-check;
    }
    // lib.optionalAttrs buildBuddyEnabled {
      gascity-buildbuddy-proxy = allServiceUsers.gascity-buildbuddy-proxy;
    };
in
{
  config = lib.mkIf cfg.enable {
    users.groups = {
      ${sharedGroup} = { };
      gascity = { };
      gascity-agent = { };
      gascity-discord = { };
      gascity-publisher = { };
      gascity-egress = { };
      gascity-agent-channel = { };
      gascity-discord-channel = { };
      gascity-publisher-channel = { };
      gascity-egress-channel = { };
      gascity-operators = { };
    }
    // lib.optionalAttrs cfg.check.enable {
      gascity-check = { };
    }
    // lib.optionalAttrs buildBuddyEnabled {
      gascity-buildbuddy-proxy = { };
    };

    users.users =
      (lib.mapAttrs mkServiceUser serviceUsers)
      // lib.genAttrs cfg.operators.users (_: {
        extraGroups = [ "gascity-operators" ];
      });

    security.sudo.extraRules = [
      {
        groups = [ "gascity-operators" ];
        runAs = "gascity";
        commands = [
          {
            command = "${cfg.package}/bin/gascity-submit";
            options = [ "NOPASSWD" ];
          }
          {
            command = "${cfg.package}/bin/gascity-status";
            options = [ "NOPASSWD" ];
          }
          {
            command = "${cfg.package}/bin/gascity-cancel";
            options = [ "NOPASSWD" ];
          }
        ];
      }
    ];

    environment.systemPackages = [ cfg.package ];

    systemd.tmpfiles.rules = [
      "d /var/lib/gascity-contributor 0710 gascity ${sharedGroup} -"
      "d /var/lib/gascity-contributor/state 0710 gascity ${sharedGroup} -"
      "d /var/lib/gascity-contributor/cache 0700 gascity gascity -"
      "d /var/lib/gascity-contributor/home 0700 gascity gascity -"
      "d /var/lib/gascity-contributor/gc 0700 gascity gascity -"
      "d /var/lib/gascity-contributor/managed 0700 gascity gascity -"
      "d /var/lib/gascity-contributor/state/rigs 0700 gascity gascity -"
      "d /var/lib/gascity-contributor/state/worktrees 0770 gascity-agent ${sharedGroup} -"
      "d /var/lib/gascity-contributor/state/leases 0700 gascity-agent gascity-agent -"
      "d /var/lib/gascity-contributor/state/agent-state 0700 gascity-agent gascity-agent -"
      "d /var/lib/gascity-contributor/state/cancellations 0770 gascity ${sharedGroup} -"
      "d /var/lib/gascity-publisher 0700 gascity-publisher gascity-publisher -"
      "d /var/cache/gascity-contributor 0700 gascity gascity -"
      "d /run/gascity-contributor 0770 root ${sharedGroup} -"
      "d /run/gascity-contributor/operator-requests 2770 root ${sharedGroup} -"
      "d /run/gascity-agent 0700 gascity-agent gascity-agent-channel -"
    ]
    ++ lib.optionals cfg.check.enable [
      "d /var/lib/gascity-check 0700 gascity-check gascity-check -"
    ];
  };
}
