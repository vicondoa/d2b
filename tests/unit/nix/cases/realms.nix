# nix-unit coverage for ADR 0043 realm option/schema foundations.
{ mkEval, lib, flakeRoot, ... }:

let
  hostBase = {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };

    d2b.site = {
      stateDir = "/var/lib/d2b";
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };

    d2b.envs.home = {
      lanSubnet = "10.10.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.envs.dev = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    d2b.envs.work = {
      lanSubnet = "10.30.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };

    d2b.vms.homebox = {
      env = "home";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.vms.devbox = {
      env = "dev";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.vms.corp = {
      env = "work";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };

  realmFixture = lib.recursiveUpdate hostBase {
    d2b.realms.home = {
      name = "Home";
      env = "home";
      network.envs = [ "home" ];
      allowedUsers = [ "alice" "alice" ];
    };

    d2b.realms.dev = {
      parent = "home";
      path = "dev.home";
      env = "dev";
      network = {
        envs = [ "work" "dev" ];
        mode = "inherit-env";
        cidrRefs = [ "lab" "dev" "lab" ];
      };
    };

    d2b.realms.work = {
      parent = "home";
      path = "work.home";
      placement = "gateway-vm";
      env = "work";
      network.envs = [ "work" ];
      providers.aca = {
        kind = "aca";
        placement = "provider-agent";
        capabilityRefs = [ "relay" "aca" "relay" ];
        configRef = "work-aca-non-secret";
      };
      relay = {
        enable = true;
        mode = "static";
        endpoints = [ "relns-b.example.invalid" "relns-a.example.invalid" ];
        credentialRef = "work-relay-credential";
      };
      policy.bundleRef = "work-policy";
      keys.enrollmentRef = "work-enrollment";
    };

    d2b.realms.archive = {
      enable = false;
      placement = "provider-specific";
      providerSpecificPlacement = "archived-off-host";
    };
  };

  cfg = (mkEval [ realmFixture ]).config;
  realms = cfg.d2b._index.realms;

  failureMessages = modules:
    map (a: a.message)
      (lib.filter (a: !a.assertion) (mkEval modules).config.assertions);

  hasMessage = needles: messages:
    lib.any
      (message: lib.all (needle: lib.hasInfix needle message) needles)
      messages;

  missingParentMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.child = {
        parent = "missing";
        path = "child.missing";
      };
    })
  ];

  parentCycleMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = {
        path = "alpha";
        parent = "beta";
      };
      d2b.realms.beta = {
        path = "beta";
        parent = "alpha";
      };
    })
  ];

  duplicateIdPathMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = {
        id = "same";
        path = "same-path";
      };
      d2b.realms.beta = {
        id = "same";
        path = "same-path";
      };
    })
  ];

  duplicateRuntimePathMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = { };
      d2b.realms.beta.paths = {
        stateDir = "/var/lib/d2b/realms/alpha";
        auditDir = "/var/lib/d2b/realms/alpha/audit";
        runDir = "/run/d2b/realms/alpha";
        publicSocket = "/run/d2b/realms/alpha/public.sock";
        brokerSocket = "/run/d2b/realms/alpha/broker.sock";
      };
    })
  ];

  legacyGatewayMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.gateways.work = {
        env = "work";
        aca.endpoint = "https://example.azurecontainerapps.invalid";
        aca.resourceGroup = "rg-example";
      };
    })
  ];

  minimalCfg = (mkEval [ (import (flakeRoot + "/examples/minimal/configuration.nix")) ]).config;
  multiEnvCfg = (mkEval [ (import (flakeRoot + "/examples/multi-env/configuration.nix")) ]).config;
in
{
  "realms/valid-home-dev-work-keeps-env-substrate-active" = {
    expr = {
      assertionsPass = lib.all (a: a.assertion) cfg.assertions;
      enabledEnvNames = cfg.d2b._index.enabledEnvNames;
      netVmByEnv = cfg.d2b._index.netVmByEnv;
      workloadNamesByEnv = cfg.d2b._index.workloadNamesByEnv;
    };
    expected = {
      assertionsPass = true;
      enabledEnvNames = [ "dev" "home" "work" ];
      netVmByEnv = {
        dev = "sys-dev-net";
        home = "sys-home-net";
        work = "sys-work-net";
      };
      workloadNamesByEnv = {
        dev = [ "devbox" ];
        home = [ "homebox" ];
        work = [ "corp" ];
      };
    };
  };

  "realms/index-normalizes-enabled-disabled-and-derived-paths" = {
    expr = {
      names = realms.names;
      enabledNames = realms.enabledNames;
      archiveInDeclared = realms.byId.archive.enabled;
      archiveInEnabled = builtins.hasAttr "archive" realms.enabledById;
      dev = {
        inherit (realms.byPath."dev.home") realmName id path pathParts parentPath parentId placement enabled;
        network = realms.byPath."dev.home".network;
      };
      home = {
        allowedUsers = realms.byPath.home.allowedUsers;
        paths = realms.byPath.home.paths;
      };
      work = {
        inherit (realms.byPath."work.home") placement;
        providerKeys = realms.byPath."work.home".providerKeys;
        enabledProviderKeys = realms.byPath."work.home".enabledProviderKeys;
        provider = realms.byPath."work.home".providers.aca;
        relay = realms.byPath."work.home".relay;
      };
      byEnv = realms.byEnv;
      bridges = {
        dev = cfg.d2b._index.envMeta.dev.lanBridge;
        home = cfg.d2b._index.envMeta.home.lanBridge;
        work = cfg.d2b._index.envMeta.work.lanBridge;
      };
    };
    expected = {
      names = [ "archive" "dev" "home" "work" ];
      enabledNames = [ "dev" "home" "work" ];
      archiveInDeclared = false;
      archiveInEnabled = false;
      dev = {
        realmName = "dev";
        id = "dev";
        path = "dev.home";
        pathParts = [ "dev" "home" ];
        parentPath = "home";
        parentId = "home";
        placement = "host-local";
        enabled = true;
        network = {
          env = "dev";
          envNames = [ "dev" "work" ];
          declaredEnvNames = [ "dev" "work" ];
          enabledEnvNames = [ "dev" "work" ];
          missingEnvNames = [ ];
          mode = "inherit-env";
          cidrRefs = [ "dev" "lab" ];
        };
      };
      home = {
        allowedUsers = [ "alice" ];
        paths = {
          stateDir = "/var/lib/d2b/realms/home";
          auditDir = "/var/lib/d2b/realms/home/audit";
          runDir = "/run/d2b/realms/home";
          publicSocket = "/run/d2b/realms/home/public.sock";
          brokerSocket = "/run/d2b/realms/home/broker.sock";
        };
      };
      work = {
        placement = "gateway-vm";
        providerKeys = [ "aca" ];
        enabledProviderKeys = [ "aca" ];
        provider = {
          providerName = "aca";
          id = "aca";
          enabled = true;
          kind = "aca";
          placement = "provider-agent";
          capabilityRefs = [ "aca" "relay" ];
          configRef = "work-aca-non-secret";
          localUnitOrdering = null;
        };
        relay = {
          enable = true;
          mode = "static";
          endpoints = [ "relns-a.example.invalid" "relns-b.example.invalid" ];
          credentialRef = "work-relay-credential";
        };
      };
      byEnv = {
        dev = {
          realmNames = [ "dev" ];
          realmIds = [ "dev" ];
          realmPaths = [ "dev.home" ];
        };
        home = {
          realmNames = [ "home" ];
          realmIds = [ "home" ];
          realmPaths = [ "home" ];
        };
        work = {
          realmNames = [ "dev" "work" ];
          realmIds = [ "dev" "work" ];
          realmPaths = [ "dev.home" "work.home" ];
        };
      };
      bridges = {
        dev = "br-dev-lan";
        home = "br-home-lan";
        work = "br-work-lan";
      };
    };
  };

  "realms/rejects-missing-parent" = {
    expr = hasMessage [
      "enabled child realms must name an enabled parent realm"
      "child.missing -> missing"
    ] missingParentMessages;
    expected = true;
  };

  "realms/rejects-parent-cycle" = {
    expr = hasMessage [
      "enabled d2b.realms parent links must form an acyclic tree"
      "alpha -> beta -> alpha"
    ] parentCycleMessages;
    expected = true;
  };

  "realms/rejects-duplicate-id-and-path" = {
    expr = {
      duplicateId = hasMessage [
        "d2b.realms must use unique stable realm ids"
        "same"
      ] duplicateIdPathMessages;
      duplicatePath = hasMessage [
        "d2b.realms must use unique canonical realm paths"
        "same-path"
      ] duplicateIdPathMessages;
    };
    expected = {
      duplicateId = true;
      duplicatePath = true;
    };
  };

  "realms/rejects-duplicate-runtime-paths" = {
    expr = {
      stateDir = hasMessage [ "must not share stateDir paths" "/var/lib/d2b/realms/alpha" ] duplicateRuntimePathMessages;
      auditDir = hasMessage [ "must not share auditDir paths" "/var/lib/d2b/realms/alpha/audit" ] duplicateRuntimePathMessages;
      runDir = hasMessage [ "must not share runDir paths" "/run/d2b/realms/alpha" ] duplicateRuntimePathMessages;
      publicSocket = hasMessage [ "must not share publicSocket paths" "/run/d2b/realms/alpha/public.sock" ] duplicateRuntimePathMessages;
      brokerSocket = hasMessage [ "must not share brokerSocket paths" "/run/d2b/realms/alpha/broker.sock" ] duplicateRuntimePathMessages;
    };
    expected = {
      stateDir = true;
      auditDir = true;
      runDir = true;
      publicSocket = true;
      brokerSocket = true;
    };
  };

  "realms/rejects-legacy-gateway-aca-surface-with-migration-guidance" = {
    expr = hasMessage [
      "legacy-surface-detected: d2b.gateways"
      "old gateway/ACA sandbox fields"
      "d2b.realms.work"
      "`d2b.envs` remains the current substrate"
    ] legacyGatewayMessages;
    expected = true;
  };

  "realms/examples-minimal-and-multi-env-still-eval" = {
    expr = {
      minimal = lib.all (a: a.assertion) minimalCfg.assertions;
      multiEnv = lib.all (a: a.assertion) multiEnvCfg.assertions;
    };
    expected = {
      minimal = true;
      multiEnv = true;
    };
  };
}
