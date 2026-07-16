{ flakeRoot, lib, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
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

  evalRealmSchema = realms:
    (lib.evalModules {
      modules = [
        (flakeRoot + "/nixos-modules/options-realms.nix")
        (flakeRoot + "/nixos-modules/index.nix")
        ({ lib, ... }: {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [ ];
          };
          config.d2b.realms = realms;
        })
      ];
    }).config.d2b._index;

  realms = {
    work = {
      path = "work.local-root";
      providers.runtime = {
        primaryAuthority = "runtime";
        implementation = "cloud-hypervisor";
      };
      workloads = {
        zeta = {
          runtime = {
            provider = "runtime";
            implementation = "qemu-media";
          };
          capabilityRefs = [ "console" ];
          roleKinds = [ "qemu-media" ];
        };
        alpha = {
          runtime = {
            provider = "runtime";
            implementation = "cloud-hypervisor";
          };
          capabilityRefs = [ "exec" "exec" ];
          launcher.capabilities = [ "display" "exec" ];
          roleKinds = [ "cloud-hypervisor" "virtiofsd" ];
        };
        archived = {
          enable = false;
          runtime.implementation = "systemd-user";
        };
      };
    };
    disabled = {
      enable = false;
      path = "disabled.local-root";
      workloads.hidden.runtime.implementation = "qemu-media";
    };
  };
  index = evalIndex realms;
  workRealmId = identity.deriveRealmId "work.local-root";
  attempt = value: (builtins.tryEval (builtins.deepSeq value true)).success;
in
{
  "realm-workloads/deterministic-order-and-enabled-filter" = {
    expr = {
      all = map (row: row.workloadName) index.workloads.list;
      enabled = map (row: row.workloadName) index.workloads.enabledList;
      work = map (row: row.workloadName)
        index.workloads.byRealmId.${workRealmId};
      targets = index.workloads.canonicalTargets;
    };
    expected = {
      all = [ "hidden" "alpha" "archived" "zeta" ];
      enabled = [ "alpha" "zeta" ];
      work = [ "alpha" "archived" "zeta" ];
      targets = [
        "hidden.disabled.local-root.d2b"
        "alpha.work.local-root.d2b"
        "archived.work.local-root.d2b"
        "zeta.work.local-root.d2b"
      ];
    };
  };

  "realm-workloads/capabilities-and-provider-bindings-are-precomputed" = {
    expr =
      let alpha = index.workloads.byCanonicalTarget."alpha.work.local-root.d2b";
      in {
        inherit (alpha) capabilityRefs providerRefs;
        label = alpha.metadata.label;
      };
    expected = {
      capabilityRefs = [ "display" "exec" ];
      providerRefs.runtime = "runtime";
      label = "alpha";
    };
  };

  "realm-workloads/explicit-and-runtime-roles-are-deduplicated" = {
    expr =
      let
        alpha = index.workloads.byCanonicalTarget."alpha.work.local-root.d2b";
        zeta = index.workloads.byCanonicalTarget."zeta.work.local-root.d2b";
      in {
        alpha = map (row: row.roleKind)
          index.roles.byWorkloadId.${alpha.workloadId};
        zeta = map (row: row.roleKind)
          index.roles.byWorkloadId.${zeta.workloadId};
      };
    expected = {
      alpha = [
        "cloud-hypervisor"
        "guest-control-health"
        "store-virtiofs-preflight"
        "virtiofsd"
        "vsock-relay"
      ];
      zeta = [ "qemu-media" ];
    };
  };

  "realm-workloads/unknown-role-kind-fails-closed" = {
    expr = attempt ((evalIndex {
      work = {
        path = "work.local-root";
        workloads.app.roleKinds = [ "raw-human-role" ];
      };
    }).identities.roleIds);
    expected = false;
  };

  "realm-workloads/realm-only-schema-feeds-normalized-index" = {
    expr =
      let
        schemaIndex = evalRealmSchema {
          local-root = {
            path = "local-root";
            providers = {
              display = {
                type = "display";
                implementationId = "wayland";
              };
              runtime = {
                type = "runtime";
                implementationId = "cloud-hypervisor";
                capabilities = [ "exec" "shell" ];
              };
            };
            workloads.app = {
              name = "Application";
              provider = "runtime";
              shell.enable = true;
              launcher = {
                enable = true;
                defaultItem = "app";
                items.app.graphical = true;
              };
            };
          };
        };
        app = builtins.head schemaIndex.workloads.list;
        runtime = lib.findFirst
          (provider: provider.providerType == "runtime")
          null
          schemaIndex.providers.list;
      in
      {
        realmId = (builtins.head schemaIndex.realms.list).realmId;
        inherit (app) canonicalTarget providerRefs workloadId;
        workloadCapabilities = app.capabilityRefs;
        workloadLabel = app.metadata.label;
        providerCapabilities = runtime.capabilityRefs;
        inherit (runtime) implementationId providerId providerType;
        roles = map (row: row.roleKind) schemaIndex.roles.list;
      };
    expected = {
      realmId = "cvudgfqzh442wwtozs7q";
      canonicalTarget = "app.local-root.d2b";
      workloadCapabilities = [ "persistent-shell" "pty" ];
      providerRefs.runtime = "runtime";
      workloadId = "n2brfnqxyvjb5iyl4vba";
      workloadLabel = "Application";
      providerCapabilities = [ "exec" "shell" ];
      implementationId = "cloud-hypervisor";
      providerId = "qbbfws6ypfhtlzgu72za";
      providerType = "runtime";
      roles = [
        "cloud-hypervisor"
        "guest-control-health"
        "store-virtiofs-preflight"
        "virtiofsd"
        "vsock-relay"
        "wayland-proxy"
      ];
    };
  };
}
