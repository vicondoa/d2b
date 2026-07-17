{ flakeRoot, lib, ... }:

let
  evalIndex = realms:
    (lib.evalModules {
      modules = [
        (flakeRoot + "/nixos-modules/index.nix")
        ({ lib, ... }: {
          options.d2b.realms = lib.mkOption {
            type = lib.types.attrs;
            default = { };
          };
          config.d2b.realms = realms;
        })
      ];
    }).config.d2b._index;

  evalAssertions = realms:
    (lib.evalModules {
      modules = [
        (flakeRoot + "/nixos-modules/index.nix")
        (flakeRoot + "/nixos-modules/assertions.nix")
        ({ lib, ... }: {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [ ];
          };
          options.d2b.realms = lib.mkOption {
            type = lib.types.attrs;
            default = { };
          };
          config.d2b.realms = realms;
        })
      ];
    }).config.assertions;

  fixture = {
    dev = {
      enable = true;
      id = "dev";
      name = "Development";
      path = "dev.local-root";
      placement = "host-local";
      providers = {
        primary = {
          enable = true;
          id = "primary";
          primaryAuthority = "runtime";
          implementation = "cloud-hypervisor";
          capabilityRefs = [ "exec" "exec" "shell" ];
          configRef = "provider-config-1";
        };
        local-storage = {
          enable = true;
          id = "local";
          primaryAuthority = "storage";
          implementation = "local";
        };
        local-transport = {
          id = "local-transport";
          primaryAuthority = "transport";
          implementation = "unix-stream";
        };
        devices = {
          id = "devices";
          primaryAuthority = "device";
          implementation = "host-mediated";
        };
        sound = {
          id = "sound";
          primaryAuthority = "audio";
          implementation = "pipewire";
        };
        wayland = {
          id = "wayland";
          primaryAuthority = "display";
          implementation = "wayland";
        };
      };
      workloads.personal-dev = {
        enable = true;
        id = "personal-dev";
        runtime = {
          provider = "primary";
          implementation = "cloud-hypervisor";
        };
        providerRefs = {
          runtime = "primary";
          device = "devices";
          audio = "sound";
          display = "wayland";
        };
        capabilityRefs = [ "guest-control" ];
        shell.enable = true;
        tpm.enable = true;
        graphics = {
          enable = true;
          videoSidecar = true;
        };
        audio.enable = true;
        usbip.enable = true;
        securityKey.enable = true;
        launcher = {
          enable = true;
          label = "Personal Development";
          capabilities = [ "guest-control" "window-forwarding" ];
        };
      };
    };
  };

  index = evalIndex fixture;
  realm = builtins.head index.realms.list;
  workload = builtins.head index.workloads.list;
  primary = index.providers.byId.f7z3k5e3awgn43aljt2a;
  cloudHypervisorRole = index.roles.byId."7xrbjonser3hpi7hqojq";
  attempt = value: (builtins.tryEval (builtins.deepSeq value true)).success;
in
{
  "index/canonical-identities-and-lookups" = {
    expr = {
      schemaVersion = index.schemaVersion;
      realm = {
        inherit (realm) realmId realmPath parentPath parentRealmId
          canonicalTargetSuffix;
      };
      workload = {
        inherit (workload) workloadId canonicalTarget capabilityRefs
          providerBindings providerRefs;
      };
      provider = {
        inherit (primary) providerId providerType implementationId capabilityRefs configRef;
      };
      role = {
        inherit (cloudHypervisorRole) roleId roleKind realmId workloadId;
      };
      byTarget = index.workloads.byCanonicalTarget.${workload.canonicalTarget}.workloadId;
    };
    expected = {
      schemaVersion = 2;
      realm = {
        realmId = "yl2hpmks5td5dkeso6qq";
        realmPath = "dev.local-root";
        parentPath = "local-root";
        parentRealmId = "cvudgfqzh442wwtozs7q";
        canonicalTargetSuffix = "dev.local-root.d2b";
      };
      workload = {
        workloadId = "q5h7jtqteem7kua4tfva";
        canonicalTarget = "personal-dev.dev.local-root.d2b";
        capabilityRefs = [
          "guest-control"
          "persistent-shell"
          "pty"
          "window-forwarding"
        ];
        providerRefs = {
          audio = "sound";
          device = "devices";
          display = "wayland";
          runtime = "primary";
        };
        providerBindings = {
          audio = {
            implementationId = "pipewire";
            providerId = "3laykn2hk5ojcazs4r4a";
            providerType = "audio";
          };
          device = {
            implementationId = "host-mediated";
            providerId = "btzjn55n4j2qrxovy3nq";
            providerType = "device";
          };
          display = {
            implementationId = "wayland";
            providerId = "4bm4k2vr7eqbzubhskjq";
            providerType = "display";
          };
          runtime = {
            implementationId = "cloud-hypervisor";
            providerId = "f7z3k5e3awgn43aljt2a";
            providerType = "runtime";
          };
        };
      };
      provider = {
        providerId = "f7z3k5e3awgn43aljt2a";
        providerType = "runtime";
        implementationId = "cloud-hypervisor";
        capabilityRefs = [ "exec" "shell" ];
        configRef = "provider-config-1";
      };
      role = {
        roleId = "7xrbjonser3hpi7hqojq";
        roleKind = "cloud-hypervisor";
        realmId = "yl2hpmks5td5dkeso6qq";
        workloadId = "q5h7jtqteem7kua4tfva";
      };
      byTarget = "q5h7jtqteem7kua4tfva";
    };
  };

  "index/enumerates-closed-role-set" = {
    expr = map (row: row.roleKind) index.roles.list;
    expected = [
      "audio"
      "cloud-hypervisor"
      "gpu"
      "gpu-render-node"
      "guest-control-health"
      "security-key-frontend"
      "store-virtiofs-preflight"
      "swtpm"
      "swtpm-pre-start-flush"
      "usbip"
      "video"
      "virtiofsd"
      "vsock-relay"
      "wayland-proxy"
    ];
  };

  "index/runtime-paths-use-only-short-identities" = {
    expr =
      let
        paths = map (row: row.path) index.storage.list;
        ids = map (row: row.resourceId) index.storage.list;
        rolePath = (builtins.head
          index.resources.byRoleId."7xrbjonser3hpi7hqojq").path;
      in
      {
        noHumanRealmName = builtins.all
          (path: !(lib.hasInfix "Development" path) && !(lib.hasInfix "/dev/" path))
          paths;
        noHumanWorkloadName = builtins.all
          (path: !(lib.hasInfix "personal-dev" path)) paths;
        noConfiguredProviderName = builtins.all
          (path: !(lib.hasInfix "primary" path)
            && !(lib.hasInfix "local-storage" path))
          paths;
        allPathsUnderFixedAnchors = builtins.all
          (path:
            lib.hasPrefix "/etc/d2b/" path
            || lib.hasPrefix "/var/lib/d2b/" path
            || lib.hasPrefix "/var/cache/d2b/" path
            || lib.hasPrefix "/run/d2b/" path)
          paths;
        resourceIdsUnique = lib.length ids == lib.length (lib.unique ids);
        inherit rolePath;
      };

    expected = {
      noHumanRealmName = true;
      noHumanWorkloadName = true;
      noConfiguredProviderName = true;
      allPathsUnderFixedAnchors = true;
      resourceIdsUnique = true;
      rolePath = "/run/d2b/r/yl2hpmks5td5dkeso6qq/w/q5h7jtqteem7kua4tfva/roles/7xrbjonser3hpi7hqojq";
    };
  };

  "index/provider-registry-mappings-have-resource-parity" = {
    expr =
      let
        mappings = index.providerRegistryV2Mappings;
        transport = builtins.head mappings.transport;
        display = builtins.head mappings.display;
        resourceIds = lib.attrNames index.resources.byId;
        displayIds = lib.attrValues display.endpointIds;
      in
      {
        transportShape = builtins.attrNames transport;
        transportIdsExist = builtins.all
          (resourceId: builtins.elem resourceId resourceIds)
          transport.transportBindingIds;
        transportResourceKinds = map
          (resourceId: index.resources.byId.${resourceId}.kind)
          transport.transportBindingIds;
        substrate = mappings.substrate;
        displayShape = builtins.attrNames display;
        displayIdsDistinct =
          lib.length displayIds == lib.length (lib.unique displayIds);
        displayIdsExist = builtins.all
          (resourceId: builtins.elem resourceId resourceIds)
          displayIds;
        displayResourceKinds = map
          (resourceId: index.resources.byId.${resourceId}.kind)
          displayIds;
        displayResourceOwners = map
          (resourceId:
            let resource = index.resources.byId.${resourceId};
            in {
              inherit (resource) providerId realmId roleId workloadId;
            })
          displayIds;
        ownerRoleKind = index.roles.byId.${display.ownerRoleId}.roleKind;
      };
    expected =
      let display = builtins.head index.providerRegistryV2Mappings.display;
      in
      {
        transportShape = [
          "controllerRole"
          "implementationId"
          "providerId"
          "realmId"
          "transportBindingIds"
        ];
        transportIdsExist = true;
        transportResourceKinds = [ "transport-binding" ];
        substrate = [ ];
        displayShape = [
          "controllerRole"
          "endpointIds"
          "implementationId"
          "ownerRoleId"
          "providerId"
          "realmId"
          "workloadId"
        ];
        displayIdsDistinct = true;
        displayIdsExist = true;
        displayResourceKinds = [
          "display-endpoint-cross-domain"
          "display-endpoint-proxy"
          "display-endpoint-wayland"
          "display-endpoint-waypipe"
        ];
        displayResourceOwners = map
          (_: {
            inherit (display) providerId realmId workloadId;
            roleId = display.ownerRoleId;
          })
          (lib.attrValues display.endpointIds);
        ownerRoleKind = "wayland-proxy";
      };
  };

  "index/provider-registry-mappings-satisfy-extension-contracts" = {
    expr =
      let
        mappings = index.providerRegistryV2Mappings;
        transportExtension = import
          (flakeRoot + "/nixos-modules/provider-registry-v2-extensions/transport.nix")
          { inherit lib; };
        displayExtension = import
          (flakeRoot + "/nixos-modules/provider-registry-v2-extensions/display.nix")
          { inherit lib; };
        transportEntry = builtins.head
          (transportExtension.mkEntries mappings.transport);
        displayEntry = builtins.head
          (displayExtension.mkEntries mappings.display);
      in
      {
        transportProviderId = transportEntry.descriptor.providerId;
        transportPlacement = transportEntry.descriptor.placement;
        transportAxis = transportEntry.binding.axis;
        transportBindingIds = transportEntry.binding.transportBindingIds;
        displayProviderId = displayEntry.descriptor.providerId;
        displayPlacement = displayEntry.descriptor.placement;
        displayAxis = displayEntry.binding.axis;
        displayWorkloadId = displayEntry.binding.workloadId;
        displayOwnerRoleId = displayEntry.binding.ownerRoleId;
        displayEndpointIds = displayEntry.binding.endpointIds;
      };
    expected =
      let
        transport = builtins.head index.providerRegistryV2Mappings.transport;
        display = builtins.head index.providerRegistryV2Mappings.display;
      in
      {
        transportProviderId = transport.providerId;
        transportPlacement = {
          kind = "trusted-first-party-in-process";
          inherit (transport) realmId controllerRole;
        };
        transportAxis = "local-transport";
        transportBindingIds = transport.transportBindingIds;
        displayProviderId = display.providerId;
        displayPlacement = {
          kind = "trusted-first-party-in-process";
          inherit (display) realmId controllerRole;
        };
        displayAxis = "local-display";
        displayWorkloadId = display.workloadId;
        displayOwnerRoleId = display.ownerRoleId;
        displayEndpointIds = display.endpointIds;
      };
  };

  "index/provider-registry-mappings-include-local-root-substrate" = {
    expr =
      let
        mappings = (evalIndex {
          local-root = {
            path = "local-root";
            providers.host = {
              type = "substrate";
              implementationId = "nixos";
            };
          };
        }).providerRegistryV2Mappings;
        substrateExtension = import
          (flakeRoot + "/nixos-modules/provider-registry-v2-extensions/substrate.nix")
          { inherit lib; };
        entry = builtins.head
          (substrateExtension.mkEntries mappings.substrate);
      in
      {
        count = lib.length mappings.substrate;
        row = builtins.head mappings.substrate;
        placement = entry.descriptor.placement;
        axis = entry.binding.axis;
      };
    expected =
      let
        localRootId = (import
          (flakeRoot + "/nixos-modules/v2-identity.nix")).deriveRealmId
            "local-root";
        providerId = (import
          (flakeRoot + "/nixos-modules/v2-identity.nix")).deriveProviderId
            localRootId "substrate" "host";
      in
      {
        count = 1;
        axis = "local-substrate";
        placement = {
          kind = "trusted-first-party-in-process";
          realmId = localRootId;
          controllerRole = "local-root-controller";
        };
        row = {
          controllerRole = "local-root-controller";
          implementationId = "nixos";
          inherit providerId;
          realmId = localRootId;
        };
      };
  };

  "index/display-mapping-requires-one-provider" = {
    expr = attempt ((evalIndex {
      dev = {
        path = "dev.local-root";
        providers.runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        workloads.app = {
          provider = "runtime";
          launcher.items.app.graphical = true;
        };
      };
    }).providerRegistryV2Mappings.display);
    expected = false;
  };

  "index/display-provider-cannot-bind-multiple-workloads" = {
    expr = attempt ((evalIndex {
      dev = {
        path = "dev.local-root";
        providers = {
          runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          wayland = {
            type = "display";
            implementationId = "wayland";
          };
        };
        workloads = {
          first = {
            provider = "runtime";
            launcher.items.app.graphical = true;
          };
          second = {
            provider = "runtime";
            launcher.items.app.graphical = true;
          };
        };
      };
    }).providerRegistryV2Mappings.display);
    expected = false;
  };

  "index/duplicate-realm-paths-fail-closed" = {
    expr = attempt ((evalIndex {
      first.path = "dev.local-root";
      second.path = "dev.local-root";
    }).identities.realmIds);
    expected = false;
  };

  "index/duplicate-provider-identities-fail-closed" = {
    expr = attempt ((evalIndex {
      dev = {
        path = "dev.local-root";
        providers = {
          first = {
            id = "primary";
            primaryAuthority = "runtime";
            implementation = "cloud-hypervisor";
          };
          second = {
            id = "primary";
            primaryAuthority = "runtime";
            implementation = "cloud-hypervisor";
          };
        };
      };
    }).identities.providerIds);
    expected = false;
  };

  "index/companion-builders-do-not-read-the-module-fixpoint" = {
    expr = builtins.all
      (path:
        !(lib.hasInfix "config.d2b._index"
          (builtins.readFile (flakeRoot + "/nixos-modules/${path}"))))
      [
        "index-realms.nix"
        "index-workloads.nix"
        "index-resources.nix"
      ];
    expected = true;
  };

  "index/realm-only-assertions-accept-normalized-fixture" = {
    expr = builtins.all (assertion: assertion.assertion)
      (evalAssertions fixture);
    expected = true;
  };

  "index/realm-only-assertions-reject-parent-cycles" = {
    expr = builtins.all (assertion: assertion.assertion) (evalAssertions {
      first = {
        path = "first.local-root";
        parent = "second.local-root";
      };
      second = {
        path = "second.local-root";
        parent = "first.local-root";
      };
    });
    expected = false;
  };

  "index/realm-only-assertions-reject-missing-provider-bindings" = {
    expr = builtins.all (assertion: assertion.assertion) (evalAssertions {
      dev = {
        path = "dev.local-root";
        workloads.app.runtime = {
          provider = "missing";
          implementation = "cloud-hypervisor";
        };
      };
    });
    expected = false;
  };

  "index/realm-only-assertions-reject-provider-type-mismatches" = {
    expr = builtins.all (assertion: assertion.assertion) (evalAssertions {
      dev = {
        path = "dev.local-root";
        providers.storage = {
          type = "storage";
          implementationId = "local";
        };
        workloads.app.provider = "storage";
      };
    });
    expected = false;
  };
}
