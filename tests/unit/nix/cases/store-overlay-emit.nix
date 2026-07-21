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
    d2b.realms.work = {
      allowedUsers = [ "alice" ];
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.editor = {
        providerRefs.runtime = "runtime";
        config = {
          networking.hostName = "editor";
          users.users.alice = { isNormalUser = true; uid = 1000; };
        };
      };
    };
  };

  cfg = (mkEval [ fixture ]).config;
  realmId = identity.deriveRealmId "work.local-root";
  workloadId = identity.deriveWorkloadId realmId "editor";
  paths = cfg.d2b._bundle.storageJson.data.paths;
  workloadPaths =
    builtins.filter (row: row.scope == "workload:${workloadId}") paths;
  pathById = id:
    lib.findFirst
      (row: row.id == id)
      (throw "store-overlay-emit: missing ${id}")
      workloadPaths;
  live = pathById "workload/${workloadId}/store-view-live";
  parent = pathById "path:workload-store-view:${workloadId}";
  meta = pathById "workload/${workloadId}/store-view-meta";
  ready = pathById "path:workload-store-ready:${workloadId}";
  dag = builtins.head
    (builtins.filter
      (row: row.vm == workloadId)
      cfg.d2b._bundle.processesJson.data.vms);
  roStore = lib.findFirst
    (node:
      node.role == "virtiofsd"
      && lib.any (arg: arg == "--shared-dir=${live.pathTemplate}") node.argv)
    (throw "store-overlay-emit: missing ro-store virtiofsd node")
    dag.nodes;
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
  "store-overlay-emit/short-id-store-view-path" = {
    expr = live.pathTemplate;
    expected =
      "/var/lib/d2b/r/${realmId}/w/${workloadId}/store-view/live";
  };

  "store-overlay-emit/live-row-is-broker-owned" = {
    expr = {
      creator = live.creator;
      inherit (live) recursive repairPolicy;
    };
    expected = {
      creator = {
        kind = "broker";
        value = "d2bbr-r-${realmId}";
      };
      recursive = false;
      repairPolicy = "broker-reconcile";
    };
  };

  "store-overlay-emit/live-hardlink-invariants" = {
    expr = hardlinkInvariants live;
    expected = true;
  };

  "store-overlay-emit/parent-hardlink-invariants" = {
    expr = hardlinkInvariants parent;
    expected = true;
  };

  "store-overlay-emit/live-never-points-at-host-store" = {
    expr =
      live.pathTemplate != "/nix/store"
      && !(lib.hasPrefix "/nix/store/" live.pathTemplate);
    expected = true;
  };

  "store-overlay-emit/meta-is-separate-from-live" = {
    expr =
      meta.pathTemplate
      == "/var/lib/d2b/r/${realmId}/w/${workloadId}/store-view/meta"
      && meta.pathTemplate != live.pathTemplate;
    expected = true;
  };

  "store-overlay-emit/ready-marker-is-read-only" = {
    expr = {
      inherit (ready) kind mode recursive;
      path = ready.pathTemplate;
    };
    expected = {
      kind = "regular-file";
      mode = "0444";
      recursive = false;
      path =
        "/var/lib/d2b/r/${realmId}/w/${workloadId}/store-view/live/.ready";
    };
  };

  "store-overlay-emit/virtiofs-serves-live-view" = {
    expr = lib.any
      (arg: arg == "--shared-dir=${live.pathTemplate}")
      roStore.argv;
    expected = true;
  };

  "store-overlay-emit/virtiofs-live-view-is-read-only" = {
    expr = builtins.elem "--readonly" roStore.argv;
    expected = true;
  };
}
