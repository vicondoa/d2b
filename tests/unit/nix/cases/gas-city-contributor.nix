# U4 eval coverage for the standalone Gas City contributor boundary.
#
# This is intentionally a Nix-unit case rather than a host test: the service
# shape, assertions, identities, credentials, quotas, and firewall text all
# render without booting systemd.  Activation-time canonical-path, file-type,
# ownership, project-quota, and free-space checks are covered by the helper
# contract and are not faked at evaluation time.
{ mkEval, lib, pkgs, flakeRoot, ... }:

let
  namedModule = import (flakeRoot + "/nixos-modules/gas-city-contributor") {
    packageFor = _: pkgs.hello;
  };

  validConfig = {
    services.gasCityContributor = {
      enable = true;
      repository.githubSlug = "vicondoa/d2b";
      repository.baseBranch = "v3";
      repository.rigName = "d2b";
      operators.users = [ "alice" ];
      credentials = {
        copilotTokenFile = "/run/secrets/gascity/copilot";
        githubPrivateKeyFile = "/run/secrets/gascity/github-app";
        discordBotTokenFile = "/run/secrets/gascity/discord";
        buildBuddyApiKeyFile = "/run/secrets/gascity/buildbuddy";
      };
      github.appId = "1234";
      github.installationId = "5678";
      discord.applicationId = "1001";
      discord.guildId = "1002";
      discord.channelId = "1003";
      discord.operatorUserIds = [ "1004" ];
      hostReadOnlyPaths = [ "/etc/hostname" ];
      check.enable = true;
      buildBuddy.enable = true;
    };
  };

  evalWith = override: mkEval [
    namedModule
    ({ ... }: override)
  ];

  disabled = (mkEval [ namedModule ]).config;
  enabled = (evalWith validConfig).config;
  main = enabled.systemd.services.gas-city-contributor.serviceConfig;
  agent = enabled.systemd.services.gascity-agent.serviceConfig;
  discordUnit = enabled.systemd.services.gascity-discord;
  discord = enabled.systemd.services.gascity-discord.serviceConfig;
  publisherUnit = enabled.systemd.services.gascity-publisher;
  publisher = enabled.systemd.services.gascity-publisher.serviceConfig;
  egress = enabled.systemd.services.gascity-egress.serviceConfig;
  check = enabled.systemd.services.gascity-check.serviceConfig;
  proxy = enabled.systemd.services.gascity-buildbuddy-proxy.serviceConfig;
  slice = enabled.systemd.slices.gascity-contributor.sliceConfig;
  firewall = enabled.networking.nftables.ruleset;
  users = enabled.users.users;
  textValue = value:
    if builtins.isList value then lib.concatStringsSep "\n" value else value;
  discordStartText = textValue discord.ExecStart;
  publisherStartText = textValue publisher.ExecStart;
  egressStartText = textValue egress.ExecStart;

  invalidPath = evalWith {
      services.gasCityContributor = validConfig.services.gasCityContributor // {
        credentials.copilotTokenFile = "relative/copilot";
      };
  };

  invalidProjection = evalWith {
      services.gasCityContributor = validConfig.services.gasCityContributor // {
        hostReadOnlyPaths = [ "/" ];
      };
  };

  invalidQuota = evalWith {
      services.gasCityContributor = validConfig.services.gasCityContributor // {
        storage.totalQuotaBytes = 230 * 1024 * 1024 * 1024;
      };
  };

  hasFailure = needle: assertions:
    lib.any (
      assertion:
      !assertion.assertion && lib.hasInfix needle assertion.message
    ) assertions;
in
{
  "gas-city-contributor/disabled-inert" = {
    expr = {
      service = disabled.systemd.services ? gas-city-contributor;
      users = disabled.users.users ? gascity;
      slice = disabled.systemd.slices ? gascity-contributor;
    };
    expected = {
      service = false;
      users = false;
      slice = false;
    };
  };

  "gas-city-contributor/service-topology" = {
    expr = {
      main = enabled.systemd.services ? gas-city-contributor;
      agent = enabled.systemd.services ? gascity-agent;
      discord = enabled.systemd.services ? gascity-discord;
      publisher = enabled.systemd.services ? gascity-publisher;
      egress = enabled.systemd.services ? gascity-egress;
      check = enabled.systemd.services ? gascity-check;
      proxy = enabled.systemd.services ? gascity-buildbuddy-proxy;
      monitor = enabled.systemd.services ? gascity-free-space-monitor;
    };
    expected = {
      main = true;
      agent = true;
      discord = true;
      publisher = true;
      egress = true;
      check = true;
      proxy = true;
      monitor = true;
    };
  };

  "gas-city-contributor/identities-and-slice" = {
    expr = {
      identities = builtins.all (name: builtins.hasAttr name users) [
        "gascity"
        "gascity-agent"
        "gascity-discord"
        "gascity-publisher"
        "gascity-egress"
        "gascity-check"
        "gascity-buildbuddy-proxy"
      ];
      mainSlice = main.Slice;
      mainGroup = main.Group;
      discordUid = users.gascity-discord.uid;
      publisherUid = users.gascity-publisher.uid;
      killMode = main.KillMode;
      cpu = slice.CPUQuota;
      memoryHigh = slice.MemoryHigh;
      memoryMax = slice.MemoryMax;
      tasks = slice.TasksMax;
    };
    expected = {
      identities = true;
      mainSlice = "gascity-contributor.slice";
      mainGroup = "gascity-agent-channel";
      discordUid = 45102;
      publisherUid = 45103;
      killMode = "control-group";
      cpu = "100%";
      memoryHigh = "25%";
      memoryMax = "30%";
      tasks = 512;
    };
  };

  "gas-city-contributor/credential-ownership" = {
    expr = {
      main = main.LoadCredential or [ ];
      agent = agent.LoadCredential or [ ];
      discord = discord.LoadCredential or [ ];
      publisher = publisher.LoadCredential or [ ];
      proxy = proxy.LoadCredential or [ ];
      check = check.LoadCredential or [ ];
    };
    expected = {
      main = [ ];
      agent = [ "copilot-token:/run/secrets/gascity/copilot" ];
      discord = [ "discord-bot-token:/run/secrets/gascity/discord" ];
      publisher = [ "github-app-private-key:/run/secrets/gascity/github-app" ];
      proxy = [ "buildbuddy-api-key:/run/secrets/gascity/buildbuddy" ];
      check = [ ];
    };
  };

  "gas-city-contributor/roots-and-quotas" = {
    expr = {
      home = builtins.elem "HOME=/var/lib/gascity-contributor/home" main.Environment;
      xdg = builtins.elem
        "XDG_CACHE_HOME=/var/cache/gascity-contributor"
        main.Environment;
      stateQuota = main.StateDirectoryQuota;
      cacheQuota = main.CacheDirectoryQuota;
      totalQuota = enabled.services.gasCityContributor.storage.totalQuotaBytes;
      discordQuota = discord.StateDirectoryQuota;
      discordStateDirectory = discord.StateDirectory;
      discordDestructiveCleanup = lib.any
        (rule: lib.hasInfix "v /var/lib/gascity-discord" rule)
        enabled.systemd.tmpfiles.rules;
      checkQuota = check.StateDirectoryQuota;
      localStore = lib.hasInfix "local?root=/var/lib/gascity-check/nix-root" (lib.concatStringsSep "\n" check.Environment);
    };
    expected = {
      home = true;
      xdg = true;
      stateQuota = "107374182400";
      cacheQuota = "26843545600";
      totalQuota = 250 * 1024 * 1024 * 1024 + 512 * 1024 * 1024;
      discordQuota = "536870912";
      discordStateDirectory = "gascity-discord";
      discordDestructiveCleanup = false;
      checkQuota = "107374182400";
      localStore = true;
    };
  };

  "gas-city-contributor/network-and-firewall" = {
    expr = {
      nftTable = lib.hasInfix "table inet gascity_contributor" firewall;
      noPublicMain = builtins.elem "any" main.IPAddressDeny;
      loopbackSupervisor = lib.hasInfix "8372" (lib.concatStringsSep "\n" main.Environment);
      loopbackDolt = lib.hasInfix "3307" (lib.concatStringsSep "\n" main.Environment);
      agentPrivateNetwork = agent.PrivateNetwork;
      discordPrivateNetwork = discord.PrivateNetwork;
      publisherPrivateNetwork = publisher.PrivateNetwork;
      proxyPrivateNetwork = proxy.PrivateNetwork;
      checkPrivateNetwork = check.PrivateNetwork;
      discordRequiresEgress = builtins.elem
        "gascity-egress.service"
        (discordUnit.requires or [ ]);
      publisherRequiresEgress = builtins.elem
        "gascity-egress.service"
        (publisherUnit.requires or [ ]);
      discordEgressGroup = builtins.elem
        "gascity-egress-channel"
        discord.SupplementaryGroups;
      publisherEgressGroup = builtins.elem
        "gascity-egress-channel"
        publisher.SupplementaryGroups;
      discordProxy = builtins.elem
        "HTTPS_PROXY=http://127.0.0.1:3128"
        discord.Environment;
      publisherProxy = builtins.elem
        "HTTPS_PROXY=http://127.0.0.1:3128"
        publisher.Environment;
      discordStartWrapper = lib.hasInfix "gascity-discord-start" discordStartText;
      publisherStartWrapper = lib.hasInfix "gascity-publisher-start" publisherStartText;
      allowedDiscordUid = lib.hasInfix "--allowed-uid 45102" egressStartText;
      allowedPublisherUid = lib.hasInfix "--allowed-uid 45103" egressStartText;
      allowedDiscordDomains =
        lib.hasInfix "discord.com" egressStartText
        && lib.hasInfix "gateway.discord.gg" egressStartText;
      allowedGithubDomains =
        lib.hasInfix "api.github.com" egressStartText
        && lib.hasInfix "github.com" egressStartText;
    };
    expected = {
      nftTable = true;
      noPublicMain = true;
      loopbackSupervisor = true;
      loopbackDolt = true;
      agentPrivateNetwork = true;
      discordPrivateNetwork = true;
      publisherPrivateNetwork = true;
      proxyPrivateNetwork = true;
      checkPrivateNetwork = true;
      discordRequiresEgress = true;
      publisherRequiresEgress = true;
      discordEgressGroup = true;
      publisherEgressGroup = true;
      discordProxy = true;
      publisherProxy = true;
      discordStartWrapper = true;
      publisherStartWrapper = true;
      allowedDiscordUid = true;
      allowedPublisherUid = true;
      allowedDiscordDomains = true;
      allowedGithubDomains = true;
    };
  };

  "gas-city-contributor/operator-rules" = {
    expr =
      let
        rules = builtins.filter (
          rule: builtins.elem "gascity-operators" (rule.groups or [ ])
        ) enabled.security.sudo.extraRules;
        rule = builtins.head rules;
      in
      {
        count = builtins.length rules;
        groups = rule.groups;
        runAs = rule.runAs;
        commands = map (command: command.command) rule.commands;
        options = map (command: command.options) rule.commands;
      };
    expected = {
      count = 1;
      groups = [ "gascity-operators" ];
      runAs = "gascity";
      commands = [
        "${pkgs.hello}/bin/gascity-submit"
        "${pkgs.hello}/bin/gascity-status"
        "${pkgs.hello}/bin/gascity-cancel"
      ];
      options = [
        [ "NOPASSWD" ]
        [ "NOPASSWD" ]
        [ "NOPASSWD" ]
      ];
    };
  };

  "gas-city-contributor/relative-credential-rejected" = {
    expr = hasFailure "credentials.copilotTokenFile" invalidPath.config.assertions;
    expected = true;
  };

  "gas-city-contributor/broad-projection-rejected" = {
    expr = hasFailure "hostReadOnlyPaths" invalidProjection.config.assertions;
    expected = true;
  };

  "gas-city-contributor/quota-overflow-rejected" = {
    expr = hasFailure "storage.totalQuotaBytes" invalidQuota.config.assertions;
    expected = true;
  };
}
