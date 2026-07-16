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
      };
      workloads.personal-dev = {
        enable = true;
        id = "personal-dev";
        runtime = {
          provider = "primary";
          implementation = "cloud-hypervisor";
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
        providerRefs.runtime = "primary";
        providerBindings.runtime = {
          implementationId = "cloud-hypervisor";
          providerId = "f7z3k5e3awgn43aljt2a";
          providerType = "runtime";
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
