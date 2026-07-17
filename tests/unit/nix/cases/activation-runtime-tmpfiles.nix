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
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.realms.home = {
      allowedUsers = [ "alice" ];
      workloads.corp = {
        kind = "local-vm";
        launcher.enable = true;
      };
    };
  };

  cfg = (mkEval [ fixture ]).config;
  realmId = identity.deriveRealmId "home.local-root";
  workloadId = identity.deriveWorkloadId realmId "corp";
  paths = cfg.d2b._bundle.storageJson.data.paths;
  locks = cfg.d2b._bundle.syncJson.data.locks;
  realmPaths = builtins.filter
    (row:
      row.scope == "realm:${realmId}"
      || row.scope == "workload:${workloadId}")
    paths;
  realmLocks = builtins.filter
    (row:
      row.scope == "realm:${realmId}"
      || row.scope == "workload:${workloadId}")
    locks;
  pathById = id:
    lib.findFirst
      (row: row.id == id)
      (throw "activation-runtime-tmpfiles: missing ${id}")
      paths;
  lockById = id:
    lib.findFirst
      (row: row.id == id)
      (throw "activation-runtime-tmpfiles: missing ${id}")
      locks;
  providerRows =
    (import ../../../../nixos-modules/provider-registry-v2-extensions/storage.nix {
      config = cfg;
      generation = 7;
      cfg = cfg.d2b;
      identity = identity;
      inherit lib;
    }).providers;
  provider = builtins.head providerRows;
  tmpfiles = cfg.systemd.tmpfiles.rules;
  activationNames = lib.attrNames cfg.system.activationScripts;
  hasPathFragment = fragment:
    lib.any (row: lib.hasInfix fragment row.pathTemplate) realmPaths;
in
{
  "activation-runtime-tmpfiles/fixed-anchors-exist" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /var/lib/d2b 0750 root d2bd -"
      "z /var/lib/d2b 0750 root d2bd -"
      "d /var/cache/d2b 0750 root d2bd -"
      "z /var/cache/d2b 0750 root d2bd -"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/no-realm-or-workload-leaves" = {
    expr = lib.all
      (rule:
        !(lib.hasInfix "/r/${realmId}" rule)
        && !(lib.hasInfix "/w/${workloadId}" rule))
      tmpfiles;
    expected = true;
  };

  "activation-runtime-tmpfiles/no-storage-or-key-repair-script" = {
    expr =
      !(builtins.elem "d2bStoreSync" activationNames)
      && !(builtins.elem "d2bStateDirAcl" activationNames)
      && !(builtins.elem "d2bVmStatePerms" activationNames)
      && !(builtins.elem "d2bMigrateOwnership" activationNames)
      && !(lib.hasInfix "ssh-keygen"
        cfg.system.activationScripts.d2bGenerateKeys.text);
    expected = true;
  };

  "activation-runtime-tmpfiles/canonical-short-ids" = {
    expr =
      builtins.stringLength realmId == 20
      && builtins.stringLength workloadId == 20
      && builtins.match "[a-z2-7]*" realmId != null
      && builtins.match "[a-z2-7]*" workloadId != null;
    expected = true;
  };

  "activation-runtime-tmpfiles/complete-storage-roots" = {
    expr = lib.all hasPathFragment [
      "/etc/d2b/r/${realmId}"
      "/var/lib/d2b/r/${realmId}"
      "/var/cache/d2b/r/${realmId}"
      "/run/d2b/r/${realmId}"
      "/var/lib/d2b/r/${realmId}/w/${workloadId}"
      "/run/d2b/r/${realmId}/w/${workloadId}"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/no-human-names-in-generated-paths" = {
    expr = lib.all
      (row:
        !(lib.hasInfix "/home/" row.pathTemplate)
        && !(lib.hasInfix "/corp/" row.pathTemplate))
      realmPaths;
    expected = true;
  };

  "activation-runtime-tmpfiles/broker-only-creation-and-repair" = {
    expr = lib.all
      (row:
        row.creator.kind == "broker"
        && builtins.elem row.repairPolicy [
          "broker-reconcile"
          "broker-fail-closed"
        ]
        && row.recursive == false)
      realmPaths;
    expected = true;
  };

  "activation-runtime-tmpfiles/audit-and-key-rows" = {
    expr = {
      realmAudit = (pathById "realm/${realmId}/audit").sensitivity;
      realmKeys = (pathById "path:realm-controller-keys:${realmId}").sensitivity;
      workloadAudit = (pathById "path:workload-audit:${workloadId}").sensitivity;
      workloadKeys = (pathById "workload/${workloadId}/keys").sensitivity;
    };
    expected = {
      realmAudit = "audit";
      realmKeys = "secret-adjacent";
      workloadAudit = "audit";
      workloadKeys = "secret-adjacent";
    };
  };

  "activation-runtime-tmpfiles/store-live-hardlink-carve-out" = {
    expr =
      let row = pathById "workload/${workloadId}/store-view-live";
      in {
        inherit (row) recursive repairPolicy;
        sameFilesystem = builtins.elem "same-filesystem" row.invariants;
        noRecursion = builtins.elem "hardlink-farm-no-recursion" row.invariants;
        noRecursiveMutation = builtins.elem "no-recursive-mutation" row.invariants;
        sourceIsHostStore = row.pathTemplate == "/nix/store";
      };
    expected = {
      recursive = false;
      repairPolicy = "broker-reconcile";
      sameFilesystem = true;
      noRecursion = true;
      noRecursiveMutation = true;
      sourceIsHostStore = false;
    };
  };

  "activation-runtime-tmpfiles/realm-locks-are-ofd-cloexec" = {
    expr = lib.all
      (row:
        row.kind == "ofd"
        && row.cloexecRequired
        && row.inheritancePolicy == "close-on-exec"
        && row.fdPassingPolicy.mechanism == "none"
        && row.adoptionPolicy == "reacquire-after-proof"
        && row.degradeScope == "realm")
      realmLocks;
    expected = true;
  };

  "activation-runtime-tmpfiles/store-lock-is-broker-owned" = {
    expr =
      let row = lockById "lock:workload-store:${workloadId}";
      in {
        inherit (row) kind cloexecRequired;
        owner = row.ownerProcess;
        path = row.pathTemplate;
      };
    expected = {
      kind = "ofd";
      cloexecRequired = true;
      owner = {
        kind = "broker";
        value = "d2bbr-r-${realmId}";
      };
      path = "/var/lib/d2b/r/${realmId}/w/${workloadId}/store-view/sync.lock";
    };
  };

  "activation-runtime-tmpfiles/storage-provider-fragment" = {
    expr = {
      count = builtins.length providerRows;
      authority = provider.descriptor.authority.type;
      implementation = provider.descriptor.implementationId;
      generation = provider.descriptor.registryGeneration;
      binding = provider.binding;
    };
    expected = {
      count = 1;
      authority = "storage";
      implementation = "local";
      generation = 7;
      binding = {
        axis = "local-storage";
        realmId = realmId;
        workloadId = workloadId;
        localStateId = "workload-${workloadId}-state-data";
        diskSetId = "workload-${workloadId}-disks";
        storeViewId = "workload-${workloadId}-store-view-live";
        closureSyncId = "workload-${workloadId}-store-view-state";
        mediaSetId = "workload-${workloadId}-media";
        resourceGeneration = 7;
      };
    };
  };

  "activation-runtime-tmpfiles/normalized-workload-resources-covered" = {
    expr =
      let
        emittedIds = map (row: row.id) realmPaths;
        normalizedIds = map (row: row.resourceId)
          cfg.d2b._index.workloads.byId.${workloadId}.resources;
      in
      lib.all (id: builtins.elem id emittedIds) normalizedIds;
    expected = true;
  };
}
