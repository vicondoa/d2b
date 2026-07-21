# Realm-native schema, normalization, and fixed control-plane coverage.
{ mkEval, lib, flakeRoot, pkgs, ... }:

let
  hostBase = {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
    };

    d2b.acceptDestructiveV2Cutover = true;
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = true;
    };
  };

  providers = {
    runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    devices = {
      type = "device";
      implementationId = "host-mediated";
    };
    display = {
      type = "display";
      implementationId = "wayland";
    };
    sound = {
      type = "audio";
      implementationId = "pipewire-vhost-user";
    };
    network = {
      type = "network";
      implementationId = "local-realm";
    };
    storage = {
      type = "storage";
      implementationId = "local";
    };
    transport = {
      type = "transport";
      implementationId = "cloud-hypervisor-vsock";
    };
  };

  providerRefs = {
    runtime = "runtime";
    device = "devices";
    display = "display";
    audio = "sound";
    network = "network";
    storage = "storage";
    transport = "transport";
  };

  realmFixture = lib.recursiveUpdate hostBase {
    d2b.realms.work = {
      path = "work";
      placement = "host-local";
      allowedUsers = [ "alice" ];
      broker = {
        enable = true;
        hostMutation = true;
      };
      inherit providers;
      network = {
        mode = "declared";
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      workloads.desktop = {
        inherit providerRefs;
        tpm.enable = true;
        graphics = {
          enable = true;
          videoSidecar = true;
        };
        audio = {
          enable = true;
          allowSpeakerByDefault = true;
        };
        usbip.enable = true;
        display.wayland = true;
        launcher = {
          enable = true;
          label = "Work desktop";
          items.terminal = {
            type = "exec";
            argv = [ "foot" ];
            graphical = true;
          };
        };
        config.users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
      workloads.entra = {
        providerRefs = {
          runtime = "runtime";
          device = "devices";
          network = "network";
          storage = "storage";
          transport = "transport";
        };
        tpm.enable = true;
        config.users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
      workloads.fido = {
        providerRefs = {
          runtime = "runtime";
          device = "devices";
        };
        securityKey.enable = true;
      };
    };
  };

  cfg = (mkEval [ realmFixture ]).config;
  workRealm = builtins.head cfg.d2b._index.realms.enabledList;
  workload = name:
    lib.findFirst (row: row.workloadName == name) null
      cfg.d2b._index.workloads.enabledList;
  desktop = workload "desktop";
  entra = workload "entra";
  fido = workload "fido";
  roleKinds = row: map (role: role.roleKind) row.roles;
  resourceKinds = name:
    map (row: row.resourceKind)
      (cfg.d2b._index.devices.byWorkloadId.${(workload name).workloadId} or [ ]);

  failureMessages = modules:
    map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion)
        (mkEval modules).config.assertions);
  hasMessage = needle: messages:
    lib.any (message: lib.hasInfix needle message) messages;

  schemaAssertionsModule = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          assertion = lib.mkOption { type = lib.types.bool; };
          message = lib.mkOption { type = lib.types.str; };
        };
      });
      default = [ ];
    };
  };
  schemaTry = module:
    builtins.tryEval (builtins.deepSeq
      (lib.evalModules {
        modules = [
          schemaAssertionsModule
          (flakeRoot + "/nixos-modules/options.nix")
          module
        ];
      }).config.d2b
      true);

  minimalCfg =
    (mkEval [ (import (flakeRoot + "/examples/minimal/configuration.nix")) ]).config;
  multiCfg =
    (mkEval [ (import (flakeRoot + "/examples/multi-env/configuration.nix")) ]).config;
  graphicsCfg =
    (mkEval [ (import (flakeRoot + "/examples/graphics-workstation/configuration.nix")) ]).config;
  entraCfg = (mkEval [
    (import (flakeRoot + "/examples/with-entra-id/configuration.nix"))
    {
      d2b.realms.work.workloads.work-entra = {
        providerRefs = {
          runtime = "runtime";
          device = "devices";
          network = "network";
          storage = "storage";
        };
        tpm.enable = true;
      };
    }
  ]).config;
in
{
  "realms/feature-rich-schema-evaluates" = {
    expr = map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion) cfg.assertions);
    expected =
      if pkgs.stdenv.hostPlatform.system == "x86_64-linux" then
        [ ]
      else
        [
          "realm workload graphics and audio roles require x86_64-linux"
          "Workload desktop.work.local-root.d2b: graphics/audio components are supported only on x86_64-linux."
        ];
  };

  "realms/typed-provider-bindings-normalize" = {
    expr = lib.mapAttrs
      (_: binding: {
        inherit (binding) implementationId providerType;
      })
      desktop.providerBindings;
    expected = {
      audio = {
        implementationId = "pipewire-vhost-user";
        providerType = "audio";
      };
      device = {
        implementationId = "host-mediated";
        providerType = "device";
      };
      display = {
        implementationId = "wayland";
        providerType = "display";
      };
      network = {
        implementationId = "local-realm";
        providerType = "network";
      };
      runtime = {
        implementationId = "cloud-hypervisor";
        providerType = "runtime";
      };
      storage = {
        implementationId = "local";
        providerType = "storage";
      };
      transport = {
        implementationId = "cloud-hypervisor-vsock";
        providerType = "transport";
      };
    };
  };

  "realms/feature-roles-are-normalized" = {
    expr = {
      desktop = roleKinds desktop;
      entra = roleKinds entra;
      fido = roleKinds fido;
    };
    expected = {
      desktop = [
        "audio"
        "cloud-hypervisor"
        "gpu"
        "gpu-render-node"
        "guest-control-health"
        "store-virtiofs-preflight"
        "swtpm"
        "swtpm-pre-start-flush"
        "usbip"
        "video"
        "virtiofsd"
        "vsock-relay"
        "wayland-proxy"
      ];
      entra = [
        "cloud-hypervisor"
        "guest-control-health"
        "store-virtiofs-preflight"
        "swtpm"
        "swtpm-pre-start-flush"
        "virtiofsd"
        "vsock-relay"
      ];
      fido = [
        "cloud-hypervisor"
        "guest-control-health"
        "security-key-frontend"
        "store-virtiofs-preflight"
        "virtiofsd"
        "vsock-relay"
      ];
    };
  };

  "realms/device-resources-cover-feature-schema" = {
    expr = {
      desktop = resourceKinds "desktop";
      entra = resourceKinds "entra";
      fido = resourceKinds "fido";
    };
    expected = {
      desktop = [ "gpu" "tpm" "usbip" "video" ];
      entra = [ "tpm" ];
      fido = [ "fido" ];
    };
  };

  "realms/resources-use-derived-identities" = {
    expr = {
      realmPath = workRealm.realmPath;
      target = desktop.canonicalTarget;
      pathsAreDerived = lib.all
        (resource:
          resource.path == null
          || lib.hasInfix "/r/${desktop.realmId}/" resource.path)
        desktop.resources;
      noConfiguredWorkloadPath = lib.all
        (resource:
          resource.path == null
          || !(lib.hasInfix "/desktop/" resource.path))
        desktop.resources;
    };
    expected = {
      realmPath = "work.local-root";
      target = "desktop.work.local-root.d2b";
      pathsAreDerived = true;
      noConfiguredWorkloadPath = true;
    };
  };

  "realms/fixed-local-root-units-only" = {
    expr =
      let
        services = lib.attrNames cfg.systemd.services;
        sockets = lib.attrNames cfg.systemd.sockets;
        d2bUnits = lib.filter (name: lib.hasPrefix "d2b" name)
          (services ++ sockets);
      in {
        localRoot = {
          d2bdService = builtins.hasAttr "d2bd" cfg.systemd.services;
          d2bdSocket = builtins.hasAttr "d2bd" cfg.systemd.sockets;
          brokerService =
            builtins.hasAttr "d2b-priv-broker" cfg.systemd.services;
          brokerSocket =
            builtins.hasAttr "d2b-priv-broker" cfg.systemd.sockets;
        };
        childRealmUnits = lib.filter
          (name:
            lib.hasPrefix "d2bd-r-" name
            || lib.hasPrefix "d2bbr-r-" name)
          d2bUnits;
        workloadUnits = lib.filter
          (name:
            lib.hasInfix desktop.workloadId name
            || lib.hasInfix entra.workloadId name
            || lib.hasInfix fido.workloadId name)
          d2bUnits;
      };
    expected = {
      localRoot = {
        d2bdService = true;
        d2bdSocket = true;
        brokerService = true;
        brokerSocket = true;
      };
      childRealmUnits = [ ];
      workloadUnits = [ ];
    };
  };

  "realms/child-controller-and-broker-are-parent-spawned" = {
    expr = map
      (row: {
        inherit (row)
          processRole
          parentSpawnRequired
          initialCgroupPlacement
          receivesSystemdListenFds
          selfBindsListener
          supervisionOwner;
      })
      cfg.d2b._realmProcessRows;
    expected = [
      {
        processRole = "controller";
        parentSpawnRequired = true;
        initialCgroupPlacement = "direct";
        receivesSystemdListenFds = false;
        selfBindsListener = false;
        supervisionOwner = "local-root-controller";
      }
      {
        processRole = "broker";
        parentSpawnRequired = true;
        initialCgroupPlacement = "direct";
        receivesSystemdListenFds = false;
        selfBindsListener = false;
        supervisionOwner = "local-root-controller";
      }
    ];
  };

  "realms/canonical-artifacts-reference-realm-control-plane" = {
    expr = {
      allocator = cfg.d2b._bundle.bundle.data.allocatorPath;
      controllers = cfg.d2b._bundle.bundle.data.realmControllersPath;
      identity = cfg.d2b._bundle.bundle.data.realmIdentityPath;
      controllerRows =
        lib.length cfg.d2b._bundle.realmControllersJson.data.controllers;
      allocatorRealmPaths =
        map (row: row.realmPath)
          cfg.d2b._bundle.allocatorJson.data.realms;
    };
    expected = {
      allocator = "/etc/d2b/allocator.json";
      controllers = "/etc/d2b/realm-controllers.json";
      identity = "/etc/d2b/realm-identity.json";
      controllerRows = 1;
      allocatorRealmPaths = [ "work.local-root" ];
    };
  };

  "realms/examples-render-feature-roles" = {
    expr =
      let
        graphics = builtins.head graphicsCfg.d2b._index.workloads.enabledList;
        # The realm sets `network.mode = "declared"`, so the auto-declared
        # net VM workload is also present in `enabledList` alongside
        # `work-entra`; select the example's own workload by name rather
        # than assuming it's the only (or first) entry.
        entraExample = lib.findFirst
          (row: row.workloadName == "work-entra")
          (throw "with-entra-id example: work-entra workload missing from enabledList")
          entraCfg.d2b._index.workloads.enabledList;
      in {
        minimalAssertions =
          lib.all (assertion: assertion.assertion) minimalCfg.assertions;
        multiAssertions =
          lib.all (assertion: assertion.assertion) multiCfg.assertions;
        graphicsAssertions =
          lib.all (assertion: assertion.assertion) graphicsCfg.assertions;
        entraAssertions =
          lib.all (assertion: assertion.assertion) entraCfg.assertions;
        graphicsRoles = roleKinds graphics;
        graphicsDevices = map (row: row.resourceKind)
          (graphicsCfg.d2b._index.devices.byWorkloadId.${graphics.workloadId} or [ ]);
        entraRoles = roleKinds entraExample;
        entraDevices = map (row: row.resourceKind)
          (entraCfg.d2b._index.devices.byWorkloadId.${entraExample.workloadId} or [ ]);
      };
    expected = {
      minimalAssertions = true;
      multiAssertions = true;
      graphicsAssertions = pkgs.stdenv.hostPlatform.system == "x86_64-linux";
      entraAssertions = true;
      graphicsRoles = [
        "audio"
        "cloud-hypervisor"
        "gpu"
        "gpu-render-node"
        "guest-control-health"
        "store-virtiofs-preflight"
        "swtpm"
        "swtpm-pre-start-flush"
        "usbip"
        "video"
        "virtiofsd"
        "vsock-relay"
        "wayland-proxy"
      ];
      graphicsDevices = [ "tpm" "gpu" "video" "usbip" ];
      entraRoles = [
        "cloud-hypervisor"
        "guest-control-health"
        "store-virtiofs-preflight"
        "swtpm"
        "swtpm-pre-start-flush"
        "virtiofsd"
        "vsock-relay"
      ];
      entraDevices = [ "tpm" ];
    };
  };

  "realms/rejects-missing-runtime-binding" = {
    expr = hasMessage
      "must bind providerRefs.runtime explicitly"
      (failureMessages [
        (lib.recursiveUpdate hostBase {
          d2b.realms.work.workloads.desktop.enable = true;
        })
      ]);
    expected = true;
  };

  "realms/rejects-unknown-provider-binding" = {
    expr = hasMessage
      "selects undeclared device provider missing"
      (failureMessages [
        (lib.recursiveUpdate hostBase {
          d2b.realms.work = {
            providers.runtime = providers.runtime;
            workloads.desktop.providerRefs = {
              runtime = "runtime";
              device = "missing";
            };
          };
        })
      ]);
    expected = true;
  };

  "realms/rejects-device-conflicts" = {
    expr = hasMessage
      "cannot request USBIP and FIDO security-key mediation simultaneously"
      (failureMessages [
        (lib.recursiveUpdate realmFixture {
          d2b.realms.work.workloads.desktop.securityKey.enable = true;
        })
      ]);
    expected = true;
  };

  "realms/rejects-unsafe-east-west-without-ack" = {
    expr = hasMessage
      "network.lan.allowEastWest requires d2b.site.allowUnsafeEastWest"
      (failureMessages [
        (lib.recursiveUpdate realmFixture {
          d2b.realms.work.network.lan.allowEastWest = true;
        })
      ]);
    expected = true;
  };

  "realms/legacy-options-are-unknown-with-no-tombstones" = {
    expr = {
      vms = (schemaTry {
        d2b.acceptDestructiveV2Cutover = true;
        d2b.vms.desktop.enable = true;
      }).success;
      envs = (schemaTry {
        d2b.acceptDestructiveV2Cutover = true;
        d2b.envs.work.enable = true;
      }).success;
      legacyWorkloadKind = (schemaTry {
        d2b.acceptDestructiveV2Cutover = true;
        d2b.realms.work.workloads.desktop = {
          kind = "local-vm";
          providerRefs.runtime = "runtime";
        };
      }).success;
      legacyVmName = (schemaTry {
        d2b.acceptDestructiveV2Cutover = true;
        d2b.realms.work.workloads.desktop = {
          legacyVmName = "desktop";
          providerRefs.runtime = "runtime";
        };
      }).success;
    };
    expected = {
      vms = false;
      envs = false;
      legacyWorkloadKind = false;
      legacyVmName = false;
    };
  };
}
