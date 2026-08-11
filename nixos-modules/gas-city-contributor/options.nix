{ config, gasCityContributorPackage, lib, ... }:

let
  inherit (lib) mkOption types;

  byteCount = types.ints.positive;
  nonNegativeByteCount = types.ints.between 0 9223372036854775807;
  identifierPattern = "^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$";
  identifier = types.strMatching identifierPattern;
  userName = types.strMatching "^[A-Za-z_][A-Za-z0-9_.-]{0,31}$";
  numericId = types.strMatching "^[0-9]+$";
  repositorySlugPattern = "^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$";
  repositorySlug = types.strMatching repositorySlugPattern;
  branchNamePattern = "^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$";
  branchName = types.strMatching branchNamePattern;
  domainPattern = types.strMatching "^(\\*\\.)?[A-Za-z0-9][A-Za-z0-9.-]{0,252}$";

  hasDotDot = value:
    lib.any (part: part == "..") (lib.splitString "/" value);

  absoluteNormalized = value:
    lib.hasPrefix "/" value
    && !(lib.hasPrefix "//" value)
    && !(hasDotDot value)
    && value != "/nix/store"
    && !(lib.hasPrefix "/nix/store/" value);

  hostProjectionSafe = value:
    absoluteNormalized value
    && value != "/"
    && value != "/home"
    && !(lib.hasPrefix "/home/" value)
    && value != "/root"
    && !(lib.hasPrefix "/root/" value)
    && value != "/etc"
    && value != "/etc/shadow"
    && value != "/etc/gshadow"
    && value != "/etc/ssh"
    && !(lib.hasPrefix "/etc/ssh/" value)
    && value != "/etc/nixos"
    && value != "/tmp"
    && !(lib.hasPrefix "/tmp/" value)
    && value != "/proc"
    && !(lib.hasPrefix "/proc/" value)
    && value != "/sys"
    && !(lib.hasPrefix "/sys/" value)
    && value != "/dev"
    && !(lib.hasPrefix "/dev/" value)
    && value != "/var"
    && value != "/var/lib"
    && !(lib.hasPrefix "/var/lib/" value)
    && value != "/var/cache"
    && !(lib.hasPrefix "/var/cache/" value)
    && value != "/var/run"
    && !(lib.hasPrefix "/var/run/" value)
    && value != "/var/lib/gascity-contributor"
    && !(lib.hasPrefix "/var/lib/gascity-contributor/" value)
    && value != "/run"
    && !(lib.hasPrefix "/run/" value)
    && !(lib.hasSuffix ".sock" value)
    && !(lib.hasSuffix "/" value);

  domainSafe = value:
    builtins.match "^(\\*\\.)?[A-Za-z0-9][A-Za-z0-9.-]{0,252}$" value != null
    && !(lib.hasInfix ".." value)
    && !(lib.hasInfix ":" value)
    && !(lib.hasInfix "/" value)
    && !(lib.hasPrefix "." value)
    && !(lib.hasSuffix "." value)
    && !(lib.hasPrefix "127." value)
    && !(lib.hasPrefix "10." value)
    && !(lib.hasPrefix "192.168." value)
    && !(lib.hasPrefix "169.254." value)
    && value != "localhost"
    && value != "metadata.google.internal";

  officialCopilotDomains = [
    "api.github.com"
    "api.githubcopilot.com"
    "copilot-proxy.githubusercontent.com"
    "github.com"
  ];
in
{
  options.services.gasCityContributor = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Run the isolated Gas City contributor environment.";
    };

    # The closure-passed package keeps this module reusable while ensuring the
    # flake's named output uses exactly the pinned contributor closure.
    package = mkOption {
      type = types.package;
      default = gasCityContributorPackage;
      readOnly = true;
      description = "Pinned Gas City contributor runtime closure.";
    };

    repository = {
      githubSlug = mkOption {
        type = types.str;
        default = "";
        description = "GitHub owner/repository used by the contributor.";
      };
      baseBranch = mkOption {
        type = types.str;
        default = "";
        description = "Pull-request base branch.";
      };
      rigName = mkOption {
        type = types.str;
        default = "d2b";
        description = "Gas City rig identifier.";
      };
    };

    operators.users = mkOption {
      type = types.listOf userName;
      default = [ ];
      description = "Local users allowed to invoke contributor wrappers.";
    };

    credentials = {
      copilotTokenFile = mkOption {
        type = types.str;
        default = "";
        description = "Root-owned source for the Copilot token.";
      };
      githubPrivateKeyFile = mkOption {
        type = types.str;
        default = "";
        description = "Root-owned source for the GitHub App private key.";
      };
      discordBotTokenFile = mkOption {
        type = types.str;
        default = "";
        description = "Root-owned source for the Discord bot token.";
      };
      buildBuddyApiKeyFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional root-owned BuildBuddy API-key source.";
      };
    };

    github = {
      appId = mkOption {
        type = types.str;
        default = "";
        description = "GitHub App identifier.";
      };
      installationId = mkOption {
        type = types.str;
        default = "";
        description = "GitHub App installation identifier.";
      };
    };

    discord = {
      applicationId = mkOption {
        type = types.str;
        default = "";
        description = "Discord application identifier.";
      };
      guildId = mkOption {
        type = types.str;
        default = "";
        description = "Discord guild identifier.";
      };
      channelId = mkOption {
        type = types.str;
        default = "";
        description = "Discord decision channel identifier.";
      };
      operatorUserIds = mkOption {
        type = types.listOf numericId;
        default = [ ];
        description = "Discord user identifiers allowed to decide.";
      };
    };

    hostReadOnlyPaths = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Narrow host paths projected read-only into the service.";
    };

    network.allowedDomains = mkOption {
      type = types.listOf domainPattern;
      default = officialCopilotDomains;
      description = "Exact or left-label wildcard domains accepted by egress.";
    };

    resources = {
      cpuQuotaPercent = mkOption {
        type = types.ints.between 1 100;
        default = 100;
        description = "Contributor slice CPU quota.";
      };
      memoryHighPercent = mkOption {
        type = types.ints.between 1 100;
        default = 25;
        description = "Contributor slice memory pressure threshold.";
      };
      memoryMaxPercent = mkOption {
        type = types.ints.between 1 100;
        default = 30;
        description = "Contributor slice hard memory ceiling.";
      };
      memorySwapMaxBytes = mkOption {
        type = nonNegativeByteCount;
        default = 0;
        description = "Contributor slice swap ceiling.";
      };
      tasksMax = mkOption {
        type = types.ints.positive;
        default = 512;
        description = "Contributor slice task limit.";
      };
      maxConcurrentAgents = mkOption {
        type = types.ints.positive;
        default = 2;
        description = "Maximum concurrent ACP agents.";
      };
      maxActiveRuns = mkOption {
        type = types.ints.positive;
        default = 1;
        description = "Maximum active workflow runs.";
      };
      maxHeavyChecks = mkOption {
        type = types.ints.positive;
        default = 1;
        description = "Maximum concurrent heavy checks.";
      };
      nixMaxJobs = mkOption {
        type = types.ints.positive;
        default = 1;
        description = "Local Nix builder job limit.";
      };
      nixBuildCores = mkOption {
        type = types.ints.positive;
        default = 2;
        description = "Cores requested per local Nix build.";
      };
    };

    storage = {
      totalQuotaBytes = mkOption {
        type = byteCount;
        default = 250 * 1024 * 1024 * 1024;
        description = "Aggregate persistent contributor quota.";
      };
      stateQuotaBytes = mkOption {
        type = byteCount;
        default = 100 * 1024 * 1024 * 1024;
        description = "State and worktree quota.";
      };
      cacheQuotaBytes = mkOption {
        type = byteCount;
        default = 25 * 1024 * 1024 * 1024;
        description = "Contributor cache quota.";
      };
      publisherQuotaBytes = mkOption {
        type = byteCount;
        default = 5 * 1024 * 1024 * 1024;
        description = "Publisher clone quota.";
      };
      checkQuotaBytes = mkOption {
        type = byteCount;
        default = 100 * 1024 * 1024 * 1024;
        description = "Local check store, output, and cache quota.";
      };
      minFreeBytes = mkOption {
        type = byteCount;
        default = 20 * 1024 * 1024 * 1024;
        description = "Host free-space reserve.";
      };
    };

    ports = {
      supervisor = mkOption {
        type = types.ints.between 1024 65535;
        default = 8372;
        description = "Loopback Gas City supervisor port.";
      };
      dolt = mkOption {
        type = types.ints.between 1024 65535;
        default = 3307;
        description = "Loopback Dolt SQL port.";
      };
    };

    check.enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable the uncredentialed local Nix check runner.";
    };

    buildBuddy.enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable the isolated BuildBuddy Envoy proxy.";
    };
  };

  config =
    let
      cfg = config.services.gasCityContributor;
      credentialSources =
        (lib.filter (value: value != "") [
          cfg.credentials.copilotTokenFile
          cfg.credentials.githubPrivateKeyFile
          cfg.credentials.discordBotTokenFile
        ])
        ++ lib.optional (cfg.credentials.buildBuddyApiKeyFile != null) cfg.credentials.buildBuddyApiKeyFile;

      pathAssertion = label: value: {
        assertion = absoluteNormalized value;
        message = "${label} must be an absolute, normalized path outside /nix/store.";
      };

      projectionConflict = projection:
        lib.any (
          source:
          projection == source || lib.hasPrefix "${projection}/" source
        ) credentialSources;

      quotaSum =
        cfg.storage.stateQuotaBytes
        + cfg.storage.cacheQuotaBytes
        + cfg.storage.publisherQuotaBytes
        + cfg.storage.checkQuotaBytes;

      requiredString = label: value: {
        assertion = value != "";
        message = "${label} must be set when services.gasCityContributor.enable is true.";
      };

      requiredNumericId = label: value: {
        assertion = builtins.match "^[0-9]+$" value != null;
        message = "${label} must contain only decimal digits.";
      };

      pathAssertions = [
        (pathAssertion "credentials.copilotTokenFile" cfg.credentials.copilotTokenFile)
        (pathAssertion "credentials.githubPrivateKeyFile" cfg.credentials.githubPrivateKeyFile)
        (pathAssertion "credentials.discordBotTokenFile" cfg.credentials.discordBotTokenFile)
      ]
      ++ lib.optional (cfg.credentials.buildBuddyApiKeyFile != null)
        (pathAssertion "credentials.buildBuddyApiKeyFile" cfg.credentials.buildBuddyApiKeyFile)
      ++ map (pathAssertion "hostReadOnlyPaths entry") cfg.hostReadOnlyPaths;
    in
    lib.mkIf cfg.enable (
      {
        assertions =
          pathAssertions
          ++ [
            (requiredString "repository.githubSlug" cfg.repository.githubSlug)
            {
              assertion = builtins.match repositorySlugPattern cfg.repository.githubSlug != null;
              message = "repository.githubSlug must be owner/repository.";
            }
            (requiredString "repository.baseBranch" cfg.repository.baseBranch)
            {
              assertion =
                builtins.match branchNamePattern cfg.repository.baseBranch != null
                && !(lib.hasInfix ".." cfg.repository.baseBranch)
                && !(lib.hasInfix "@{" cfg.repository.baseBranch)
                && !(lib.hasPrefix "/" cfg.repository.baseBranch)
                && !(lib.hasSuffix "/" cfg.repository.baseBranch);
              message = "repository.baseBranch is malformed.";
            }
            {
              assertion = builtins.match identifierPattern cfg.repository.rigName != null;
              message = "repository.rigName is malformed.";
            }
            {
              assertion = cfg.operators.users != [ ];
              message = "operators.users must not be empty.";
            }
            {
              assertion = lib.all (user: !(builtins.elem user [
                "gascity"
                "gascity-agent"
                "gascity-discord"
                "gascity-publisher"
                "gascity-egress"
                "gascity-check"
                "gascity-buildbuddy-proxy"
              ])) cfg.operators.users;
              message = "operators.users must not contain contributor service identities.";
            }
            (requiredNumericId "github.appId" cfg.github.appId)
            (requiredNumericId "github.installationId" cfg.github.installationId)
            (requiredNumericId "discord.applicationId" cfg.discord.applicationId)
            (requiredNumericId "discord.guildId" cfg.discord.guildId)
            (requiredNumericId "discord.channelId" cfg.discord.channelId)
            {
              assertion = cfg.discord.operatorUserIds != [ ];
              message = "discord.operatorUserIds must not be empty.";
            }
            {
              assertion = cfg.network.allowedDomains != [ ];
              message = "network.allowedDomains must not be empty.";
            }
            {
              assertion =
                lib.unique (map builtins.baseNameOf cfg.hostReadOnlyPaths)
                == (map builtins.baseNameOf cfg.hostReadOnlyPaths);
              message = "hostReadOnlyPaths entries must have distinct projection names.";
            }
            {
              assertion = cfg.resources.memoryHighPercent < cfg.resources.memoryMaxPercent;
              message = "resources.memoryHighPercent must be lower than memoryMaxPercent.";
            }
            {
              assertion = quotaSum <= cfg.storage.totalQuotaBytes;
              message = "persistent service quotas exceed storage.totalQuotaBytes.";
            }
            {
              assertion = cfg.ports.supervisor != cfg.ports.dolt;
              message = "ports.supervisor and ports.dolt must be distinct.";
            }
            {
              assertion = cfg.resources.maxActiveRuns <= cfg.resources.maxConcurrentAgents;
              message = "maxActiveRuns cannot exceed maxConcurrentAgents.";
            }
            {
              assertion = !cfg.buildBuddy.enable || cfg.credentials.buildBuddyApiKeyFile != null;
              message = "BuildBuddy requires credentials.buildBuddyApiKeyFile.";
            }
          ]
          ++ map (
            projection: {
              assertion = hostProjectionSafe projection && !(projectionConflict projection);
              message = "hostReadOnlyPaths contains an unsafe or credential-bearing projection.";
            }
          ) cfg.hostReadOnlyPaths
          ++ map (
            domain: {
              assertion = domainSafe domain;
              message = "network.allowedDomains contains an empty, literal-IP, private, or malformed domain.";
            }
          ) cfg.network.allowedDomains;
      }
    );
}
