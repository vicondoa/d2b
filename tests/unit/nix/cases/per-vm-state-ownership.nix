# Realm-native successor coverage for the retired per-VM ownership gate.
{ mkEval, lib, ... }:

let
  identity = import ../../../../nixos-modules/v2-identity.nix;
  fixture = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.acceptDestructiveV2Cutover = true;
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.realms.home = {
      allowedUsers = [ "alice" ];
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.corp = {
        providerRefs.runtime = "runtime";
        tpm.enable = true;
        config = {
          networking.hostName = "corp";
          users.users.alice = { isNormalUser = true; uid = 1000; };
        };
      };
    };
  };

  cfg = (mkEval [ fixture ]).config;
  realmId = identity.deriveRealmId "home.local-root";
  workloadId = identity.deriveWorkloadId realmId "corp";
  tpmRoleId = identity.deriveRoleId realmId workloadId "swtpm";
  paths = cfg.d2b._bundle.storageJson.data.paths;
  workloadPaths =
    builtins.filter (row: row.scope == "workload:${workloadId}") paths;
  pathById = id:
    lib.findFirst
      (row: row.id == id)
      (throw "per-vm-state-ownership: missing realm-native row ${id}")
      paths;
  workloadPathById = id:
    lib.findFirst
      (row: row.id == id)
      (throw "per-vm-state-ownership: missing workload row ${id}")
      workloadPaths;
  broker = {
    kind = "broker";
    value = "d2bbr-r-${realmId}";
  };
  hardlinkInvariants = row:
    lib.all
      (invariant: builtins.elem invariant row.invariants)
      [
        "same-filesystem"
        "hardlink-farm-no-recursion"
        "no-recursive-mutation"
      ];
in
{
  "per-vm-state-ownership/uses-short-id-scopes" = {
    expr = lib.all
      (row:
        row.scope == "realm:${realmId}"
        || row.scope == "workload:${workloadId}"
        || row.scope == "host")
      (builtins.filter
        (row:
          row.scope == "realm:${realmId}"
          || row.scope == "workload:${workloadId}"
          || row.id == "path:realm-runtime-root")
        paths);
    expected = true;
  };

  "per-vm-state-ownership/no-legacy-vm-or-env-scopes" = {
    expr = lib.all
      (row:
        !(lib.hasPrefix "vm:" row.scope)
        && !(lib.hasPrefix "env:" row.scope))
      paths;
    expected = true;
  };

  "per-vm-state-ownership/no-human-names-in-paths" = {
    expr = lib.all
      (row:
        !(lib.hasInfix "/home/" row.pathTemplate)
        && !(lib.hasInfix "/corp/" row.pathTemplate))
      paths;
    expected = true;
  };

  "per-vm-state-ownership/all-realm-rows-broker-created" = {
    expr = lib.all
      (row: row.creator.kind == "broker")
      paths;
    expected = true;
  };

  "per-vm-state-ownership/all-realm-rows-non-recursive" = {
    expr = lib.all (row: row.recursive == false) paths;
    expected = true;
  };

  "per-vm-state-ownership/workload-state-root" = {
    expr =
      let row = workloadPathById "workload/${workloadId}/state";
      in {
        inherit (row) pathTemplate owner group mode creator repairPolicy;
      };
    expected = {
      pathTemplate = "/var/lib/d2b/r/${realmId}/w/${workloadId}";
      owner = {
        kind = "user";
        value = "d2bd-r-${realmId}";
      };
      group = {
        kind = "group";
        value = "d2bcg-r-${realmId}";
      };
      mode = "0750";
      creator = broker;
      repairPolicy = "broker-reconcile";
    };
  };

  "per-vm-state-ownership/tpm-state-is-role-owned" = {
    expr =
      let row = workloadPathById "workload/${workloadId}/tpm";
      in {
        inherit (row) owner group mode repairPolicy sensitivity;
      };
    expected = {
      owner = {
        kind = "role";
        value = tpmRoleId;
      };
      group = {
        kind = "role";
        value = tpmRoleId;
      };
      mode = "0700";
      repairPolicy = "broker-fail-closed";
      sensitivity = "secret-adjacent";
    };
  };

  "per-vm-state-ownership/store-parent-hardlink-carve-out" = {
    expr =
      hardlinkInvariants
        (pathById "path:workload-store-view:${workloadId}");
    expected = true;
  };

  "per-vm-state-ownership/store-live-hardlink-carve-out" = {
    expr =
      let row = workloadPathById
        "workload/${workloadId}/store-view-live";
      in {
        invariants = hardlinkInvariants row;
        inherit (row) pathTemplate recursive creator repairPolicy;
      };
    expected = {
      invariants = true;
      pathTemplate =
        "/var/lib/d2b/r/${realmId}/w/${workloadId}/store-view/live";
      recursive = false;
      creator = broker;
      repairPolicy = "broker-reconcile";
    };
  };

  "per-vm-state-ownership/store-ready-marker-is-read-only" = {
    expr =
      let row = pathById "path:workload-store-ready:${workloadId}";
      in {
        inherit (row) kind mode recursive;
      };
    expected = {
      kind = "regular-file";
      mode = "0444";
      recursive = false;
    };
  };

  "per-vm-state-ownership/guest-session-credential-preserved" = {
    expr =
      let
        directory =
          pathById "path:workload-guest-session:${workloadId}";
        credential =
          pathById "path:workload-guest-session-credential:${workloadId}";
      in {
        directoryMode = directory.mode;
        credentialMode = credential.mode;
        credentialKind = credential.kind;
        group = credential.group;
        creator = credential.creator;
        repairPolicy = credential.repairPolicy;
        recursive = credential.recursive;
      };
    expected = {
      directoryMode = "0750";
      credentialMode = "0440";
      credentialKind = "regular-file";
      group = {
        kind = "group";
        value = "d2b-gctlfs-${workloadId}";
      };
      creator = broker;
      repairPolicy = "broker-fail-closed";
      recursive = false;
    };
  };
}
