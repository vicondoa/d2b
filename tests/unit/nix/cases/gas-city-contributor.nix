# U4 eval coverage for the standalone Gas City contributor boundary.
#
# This is intentionally a Nix-unit case rather than a host test: the service
# shape, assertions, identities, credentials, quotas, and firewall text all
# render without booting systemd.  Activation-time canonical-path, file-type,
# ownership, project-quota, and free-space checks are covered by the helper
# contract and are not faked at evaluation time.
{ mkEval, lib, pkgs, flakeRoot, ... }:

let
  testPackage = pkgs.hello // {
    passthru = (pkgs.hello.passthru or { }) // {
      runtimeScripts = pkgs.hello;
    };
  };
  namedModule = import (flakeRoot + "/nixos-modules/gas-city-contributor") {
    packageFor = _: testPackage;
  };

  validConfig = {
    services.gasCityContributor = {
      enable = true;
      repository.githubSlug = "acme/project";
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
      resources.maxHeavyChecks = 2;
      resources.checkTimeoutSeconds = 17;
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
  checkUnit = enabled.systemd.services.gascity-check;
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

  withoutBuildBuddy = (evalWith {
    services.gasCityContributor =
      validConfig.services.gasCityContributor
      // {
        credentials =
          validConfig.services.gasCityContributor.credentials
          // {
            buildBuddyApiKeyFile = null;
          };
        buildBuddy =
          validConfig.services.gasCityContributor.buildBuddy
          // {
            enable = false;
          };
      };
  }).config;
  checkWithoutBuildBuddyUnit = withoutBuildBuddy.systemd.services.gascity-check;
  checkWithoutBuildBuddy = checkWithoutBuildBuddyUnit.serviceConfig;

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
      mainGroup = "gascity-contributor";
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
      home = builtins.elem
        "HOME=/var/lib/gascity-contributor/state/home"
        main.Environment;
      xdgConfig = builtins.elem
        "XDG_CONFIG_HOME=/var/lib/gascity-contributor/state/home/.config"
        main.Environment;
      xdgState = builtins.elem
        "XDG_STATE_HOME=/var/lib/gascity-contributor/state/home/.local/state"
        main.Environment;
      gcHome = builtins.elem
        "GC_HOME=/var/lib/gascity-contributor/state/gc"
        main.Environment;
      xdg = builtins.elem
        "XDG_CACHE_HOME=/var/cache/gascity-contributor"
        main.Environment;
      stateQuota = main.StateDirectoryQuota;
      stateDirectory = main.StateDirectory;
      cacheQuota = main.CacheDirectoryQuota;
      cacheDirectory = main.CacheDirectory;
      mainReadWritePaths = main.ReadWritePaths;
      totalQuota = enabled.services.gasCityContributor.storage.totalQuotaBytes;
      discordQuota = discord.StateDirectoryQuota;
      discordStateDirectory = discord.StateDirectory;
      discordDestructiveCleanup = lib.any
        (rule: lib.hasInfix "v /var/lib/gascity-discord" rule)
        enabled.systemd.tmpfiles.rules;
      checkQuota = check.StateDirectoryQuota;
      localStore = lib.hasInfix "local?root=/var/lib/gascity-check/nix-root" (lib.concatStringsSep "\n" check.Environment);
      checkTimeout = lib.hasInfix "--timeout-seconds 17" (textValue check.ExecStart);
      checkConcurrency = lib.hasInfix "--max-heavy-checks 2" (textValue check.ExecStart);
      checkSocket = lib.hasInfix "--socket /run/gascity-check/check.sock" (textValue check.ExecStart);
      mainCheckSocket = lib.hasInfix
        "GC_CHECK_SOCKET=/run/gascity-check/check.sock"
        (lib.concatStringsSep "\n" main.Environment);
      mainAgentSocket = lib.hasInfix
        "GC_AGENT_LAUNCHER_SOCKET=/run/gascity-agent/agent.sock"
        (lib.concatStringsSep "\n" main.Environment);
      mainDiscordSocket = lib.hasInfix
        "GC_DISCORD_CHANNEL_SOCKET=/run/gascity-discord/discord.sock"
        (lib.concatStringsSep "\n" main.Environment);
      mainPublisherSocket = lib.hasInfix
        "GC_PUBLISHER_CHANNEL_SOCKET=/run/gascity-publisher/publisher.sock"
        (lib.concatStringsSep "\n" main.Environment);
      mainEgressSocket = lib.hasInfix
        "GC_EGRESS_SOCKET=/run/gascity-egress/egress.sock"
        (lib.concatStringsSep "\n" main.Environment);
      mainGeneration = lib.any
        (entry: lib.hasPrefix "GC_CITY_GENERATION=" entry)
        main.Environment;
      mainStateSchema = builtins.elem "GC_STATE_SCHEMA=1" main.Environment;
      agentStateMode = builtins.elem
        "d /var/lib/gascity-contributor/state/agent-state 0710 gascity-agent gascity-contributor -"
        enabled.systemd.tmpfiles.rules;
      terminalStateMode = builtins.elem
        "d /var/lib/gascity-contributor/state/agent-state/terminal 0750 gascity gascity-contributor -"
        enabled.systemd.tmpfiles.rules;
      managedAssetDirectory = builtins.elem
        "d /var/lib/gascity-contributor/managed 0750 root gascity-contributor -"
        enabled.systemd.tmpfiles.rules;
      managedParentDirectory = builtins.elem
        "d /var/lib/gascity-contributor 0750 root gascity-contributor -"
        enabled.systemd.tmpfiles.rules;
      stateHomeDirectory = builtins.elem
        "d /var/lib/gascity-contributor/state/home 0700 gascity gascity -"
        enabled.systemd.tmpfiles.rules;
      stateGcDirectory = builtins.elem
        "d /var/lib/gascity-contributor/state/gc 0700 gascity gascity -"
        enabled.systemd.tmpfiles.rules;
      obsoleteHomeDirectory = builtins.elem
        "d /var/lib/gascity-contributor/home 0700 gascity gascity -"
        enabled.systemd.tmpfiles.rules;
      obsoleteGcDirectory = builtins.elem
        "d /var/lib/gascity-contributor/gc 0700 gascity gascity -"
        enabled.systemd.tmpfiles.rules;
      materializeExpectedUid = lib.any
        (entry: lib.hasInfix "--uid 0" entry)
        main.ExecStartPre;
      materializeExpectedGroup = lib.any
        (entry:
          lib.hasInfix "--group" entry
          && lib.hasInfix "gascity-contributor" entry)
        main.ExecStartPre;
      serviceManagedGroup = builtins.all
        (unit:
          builtins.elem
            "gascity-contributor"
            (unit.SupplementaryGroups or [ ]))
        [ main agent discord publisher egress check proxy ];
      gcRootMode = builtins.elem
        "d /nix/var/nix/gcroots/gascity-contributor 0700 gascity-agent gascity-agent -"
        enabled.systemd.tmpfiles.rules;
      gcRootWrite = builtins.elem
        "/nix/var/nix/gcroots/gascity-contributor"
        agent.ReadWritePaths;
      agentServerUid = lib.hasInfix
        "GC_AGENT_SERVER_UID=45101"
        (lib.concatStringsSep "\n" main.Environment);
    };
    expected = {
      home = true;
      xdgConfig = true;
      xdgState = true;
      gcHome = true;
      xdg = true;
      stateQuota = "107374182400";
      stateDirectory = "gascity-contributor/state";
      cacheQuota = "26843545600";
      cacheDirectory = "gascity-contributor";
      mainReadWritePaths = [
        "/var/lib/gascity-contributor/state"
        "/run/gascity-contributor"
      ];
      totalQuota = 250 * 1024 * 1024 * 1024 + 512 * 1024 * 1024;
      discordQuota = "536870912";
      discordStateDirectory = "gascity-discord";
      discordDestructiveCleanup = false;
      checkQuota = "107374182400";
      localStore = true;
      checkTimeout = true;
      checkConcurrency = true;
      checkSocket = true;
      mainCheckSocket = true;
      mainAgentSocket = true;
      mainDiscordSocket = true;
      mainPublisherSocket = true;
      mainEgressSocket = true;
      mainGeneration = true;
      mainStateSchema = true;
      agentStateMode = true;
      terminalStateMode = true;
      managedAssetDirectory = true;
      managedParentDirectory = true;
      stateHomeDirectory = true;
      stateGcDirectory = true;
      obsoleteHomeDirectory = false;
      obsoleteGcDirectory = false;
      materializeExpectedUid = true;
      materializeExpectedGroup = true;
      serviceManagedGroup = true;
      gcRootMode = true;
      gcRootWrite = true;
      agentServerUid = true;
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
      checkJoinsNamespaceOf = checkUnit.unitConfig.JoinsNamespaceOf or [ ];
      checkServiceJoinsNamespaceOf = check ? JoinsNamespaceOf;
      checkRequires = checkUnit.requires;
      checkAfter = checkUnit.after;
      checkBindsTo = checkUnit.bindsTo;
      checkPartOf = checkUnit.unitConfig.PartOf;
      checkBefore = checkUnit.unitConfig.Before;
      checkProxy = lib.hasInfix
        "--proxy http://127.0.0.1:3128"
        (textValue check.ExecStart);
      proxyListen = lib.hasInfix
        "--listen 127.0.0.1:19801"
        (textValue proxy.ExecStart);
      checkWithoutBuildBuddyJoinsNamespaceOf =
        checkWithoutBuildBuddyUnit.unitConfig ? JoinsNamespaceOf;
      checkWithoutBuildBuddyServiceJoinsNamespaceOf =
        checkWithoutBuildBuddy ? JoinsNamespaceOf;
      buildBuddyDisabledProxy =
        withoutBuildBuddy.systemd.services ? gascity-buildbuddy-proxy;
      checkGroup = check.Group;
      checkChannelGroup = builtins.elem
        "gascity-check-channel"
        check.SupplementaryGroups;
      mainCheckChannelGroup = builtins.elem
        "gascity-check-channel"
        main.SupplementaryGroups;
      discordRequiresEgress = builtins.elem
        "gascity-egress.service"
        (discordUnit.requires or [ ]);
      publisherRequiresEgress = builtins.elem
        "gascity-egress.service"
        (publisherUnit.requires or [ ]);
      monitorRequiresMain = builtins.elem
        "gascity-free-space-monitor.service"
        (enabled.systemd.services.gas-city-contributor.requires or [ ]);
      monitorBeforeMain = builtins.elem
        "gas-city-contributor.service"
        (enabled.systemd.services.gascity-free-space-monitor.before or [ ]);
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
      mainSystemCallFilter = main.SystemCallFilter;
      proxySystemCallFilter = proxy.SystemCallFilter;
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
      checkJoinsNamespaceOf = [ "gascity-buildbuddy-proxy.service" ];
      checkServiceJoinsNamespaceOf = false;
      checkRequires = [
        "gascity-egress.service"
        "gascity-free-space-monitor.service"
        "gascity-buildbuddy-proxy.service"
      ];
      checkAfter = [
        "gascity-egress.service"
        "gascity-free-space-monitor.service"
        "gascity-buildbuddy-proxy.service"
      ];
      checkBindsTo = [ "gascity-free-space-monitor.service" ];
      checkPartOf = "gas-city-contributor.service";
      checkBefore = "gas-city-contributor.service";
      checkProxy = true;
      proxyListen = true;
      checkWithoutBuildBuddyJoinsNamespaceOf = false;
      checkWithoutBuildBuddyServiceJoinsNamespaceOf = false;
      buildBuddyDisabledProxy = false;
      checkGroup = "gascity-check-channel";
      checkChannelGroup = true;
      mainCheckChannelGroup = true;
      monitorRequiresMain = true;
      monitorBeforeMain = true;
      discordRequiresEgress = true;
      publisherRequiresEgress = true;
      discordEgressGroup = true;
      publisherEgressGroup = true;
      discordProxy = true;
      publisherProxy = true;
      mainSystemCallFilter = [
        "@system-service"
        "~@privileged"
        "~@mount"
        "~@raw-io"
        "chown"
      ];
      proxySystemCallFilter = [
        "@system-service"
        "~@privileged"
        "~@mount"
        "~@raw-io"
        "chown"
        "mincore"
      ];
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
