# nix-unit coverage for realm-owned workload index, launcher metadata, and
# cross-realm assertion contracts introduced in Wave 14.
#
# Coverage:
#   • Accepted workload config shapes (legacyVmName present, provider-placeholder, disabled)
#   • Workload index row rendering: targetAddress, runtimeKind, substrateId,
#     capabilityRefs sorted+deduped, all/enabled/byVm accessors
#   • realm-workloads-launcher.json emitter: schemaVersion, runtimeState,
#     per-workload fields, vsockCid advisory, invariants block
#   • Bundle artifact registration: installFileName, classification,
#     sensitivity, /etc install mode
#   • Cross-realm vsock CID collision assertion: fires when two workloads in
#     different realms reference different VMs with the same derived CID;
#     same-VM cross-realm references do NOT trigger the assertion
#   • Cross-realm external-network attachment conflict: advisory (assertion
#     stays true) but index.realms.externalNetworkConflicts is populated
{ mkEval, lib, ... }:

let
  # ── shared base ────────────────────────────────────────────────────────────
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
    d2b.vms.corpbox = {
      env = "work";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };

  # ── workload fixture ────────────────────────────────────────────────────────
  # One realm ("work.home") with two workloads: one with a vmRef, one without.
  workloadFixture = lib.recursiveUpdate hostBase {
    d2b.realms.home = {
      name = "Home";
      env = "home";
      network.envs = [ "home" ];
      allowedUsers = [ "alice" ];
    };
    d2b.realms.work = {
      parent = "home";
      path = "work.home";
      placement = "gateway-vm";
      env = "work";
      network.envs = [ "work" ];
      workloads.corp-laptop = {
        enable = true;
        kind = "local-vm";
        legacyVmName = "corpbox";
        launcher = {
          label = "Corp Laptop";
          icon.id = "computer-laptop";
          capabilities = [ "guest-exec" "graphics" "guest-exec" ];
        };
      };
      workloads.provider-service = {
        enable = true;
        kind = "provider-placeholder";
        launcher = {
          label = "Provider Service";
          capabilities = [ ];
        };
      };
      workloads.archived = {
        enable = false;
        kind = "local-vm";
        legacyVmName = "corpbox";
        launcher.label = "Archived";
      };
    };
  };

  wlCfg = (mkEval [ workloadFixture ]).config;
  wlIndex = wlCfg.d2b._index;
  workRealm = wlIndex.realms.byPath."work.home";

  # ── helpers ─────────────────────────────────────────────────────────────────
  failureMessages = modules:
    map (a: a.message)
      (lib.filter (a: !a.assertion) (mkEval modules).config.assertions);

  hasMessage = needles: messages:
    lib.any
      (message: lib.all (needle: lib.hasInfix needle message) needles)
      messages;

  # ── CID collision fixture ────────────────────────────────────────────────────
  # Use two envless VMs whose MD5-based CID formula produces the same value.
  # svc2532 and svc4319 both hash to md5-prefix 0x69b166 → CID 6930790.
  # Envless VMs use the hash-based CID formula (index is irrelevant).
  cidCollisionFixture = lib.recursiveUpdate hostBase {
    d2b.vms.svc2532 = {
      # no env → hash-based CID derivation
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.vms.svc4319 = {
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.realms.alpha = {
      path = "alpha";
      network.envs = [ ];
      workloads.wl-a = {
        legacyVmName = "svc2532";
        launcher.label = "Service A";
      };
    };
    d2b.realms.beta = {
      path = "beta";
      network.envs = [ ];
      workloads.wl-b = {
        legacyVmName = "svc4319";
        launcher.label = "Service B";
      };
    };
  };

  cidCollisionMessages = failureMessages [ cidCollisionFixture ];

  # ── same-VM cross-realm (no collision) fixture ──────────────────────────────
  sameVmTwoRealmsFixture = lib.recursiveUpdate hostBase {
    d2b.realms.realm-a = {
      path = "realm-a";
      env = "home";
      network.envs = [ "home" ];
      workloads.home-main = {
        legacyVmName = "homebox";
        launcher.label = "Home Main";
      };
    };
    d2b.realms.realm-b = {
      path = "realm-b";
      env = "home";
      network.envs = [ "home" ];
      workloads.home-alias = {
        legacyVmName = "homebox";
        launcher.label = "Home Alias";
      };
    };
  };

  sameVmCfg = (mkEval [ sameVmTwoRealmsFixture ]).config;
  sameVmMessages = failureMessages [ sameVmTwoRealmsFixture ];

  # ── external-network attachment conflict fixture ─────────────────────────────
  # Two realms both associate with the "work" env which has attachment enabled.
  # Only attachment is needed for conflict detection; no egress required.
  extNetConflictFixture = lib.recursiveUpdate hostBase {
    d2b.envs.work.externalNetwork.attachment = {
      enable = true;
      interface = "eth0";
    };
    d2b.realms.realm-a = {
      path = "realm-a";
      env = "work";
      network.envs = [ "work" ];
    };
    d2b.realms.realm-b = {
      path = "realm-b";
      env = "work";
      network.envs = [ "work" ];
    };
  };

  extNetCfg = (mkEval [ extNetConflictFixture ]).config;
  extNetMessages = failureMessages [ extNetConflictFixture ];
in
{
  # ── workload index: accepted config with legacyVmName ─────────────────────
  "realm-workloads/index-accepted-with-legacyvmname" = {
    expr =
      let
        row = lib.findFirst
          (w: w.workloadName == "corp-laptop")
          null
          workRealm.workloads;
      in {
        workloadName = row.workloadName;
        realmName = row.realmName;
        realmPath = row.realmPath;
        targetAddress = row.targetAddress;
        substrateId = row.substrateId;
        legacyVmName = row.legacyVmName;
        runtimeKind = row.runtimeKind;
        runtimeProviderId = row.runtimeProviderId;
        label = row.label;
        icon = row.icon;
        actionId = row.actionId;
        # capabilityRefs must be sorted and deduplicated
        capabilityRefs = row.capabilityRefs;
        enable = row.enable;
      };
    expected = {
      workloadName = "corp-laptop";
      realmName = "work";
      realmPath = "work.home";
      targetAddress = "corp-laptop.work.home.d2b";
      substrateId = "corpbox";
      legacyVmName = "corpbox";
      runtimeKind = "nixos";
      runtimeProviderId = "local-cloud-hypervisor";
      label = "Corp Laptop";
      icon = "computer-laptop";
      actionId = "corp-laptop";
      capabilityRefs = [ "graphics" "guest-exec" ];
      enable = true;
    };
  };

  # ── workload index: accepted provider-placeholder (no legacyVmName) ─────────
  "realm-workloads/index-accepted-provider-placeholder" = {
    expr =
      let
        row = lib.findFirst
          (w: w.workloadName == "provider-service")
          null
          workRealm.workloads;
      in {
        workloadName = row.workloadName;
        legacyVmName = row.legacyVmName;
        substrateId = row.substrateId;
        runtimeKind = row.runtimeKind;
        runtimeProviderId = row.runtimeProviderId;
        enable = row.enable;
      };
    expected = {
      workloadName = "provider-service";
      legacyVmName = null;
      substrateId = null;
      runtimeKind = null;
      runtimeProviderId = null;
      enable = true;
    };
  };

  # ── workload index: disabled workload excluded from enabled ─────────────────
  "realm-workloads/disabled-excluded-from-enabled" = {
    expr =
      let
        allNames = map (w: w.workloadName) workRealm.workloads;
        enabledNames = workRealm.enabledWorkloadNames;
      in {
        archivedInAll = builtins.elem "archived" allNames;
        archivedInEnabled = builtins.elem "archived" enabledNames;
        enabledCount = builtins.length enabledNames;
      };
    expected = {
      archivedInAll = true;
      archivedInEnabled = false;
      enabledCount = 2;
    };
  };

  # ── workload index: flat enabled workloads accessor ─────────────────────────
  "realm-workloads/index-flat-enabled-accessor" = {
    expr =
      let
        allEnabled = wlIndex.realms.workloads.enabled;
        enabledNames = map (w: w.workloadName) allEnabled;
      in {
        containsCorpLaptop = builtins.elem "corp-laptop" enabledNames;
        containsProviderService = builtins.elem "provider-service" enabledNames;
        doesNotContainArchived = !(builtins.elem "archived" enabledNames);
      };
    expected = {
      containsCorpLaptop = true;
      containsProviderService = true;
      doesNotContainArchived = true;
    };
  };

  # ── workload index: all accessor includes disabled ───────────────────────────
  "realm-workloads/index-all-includes-disabled" = {
    expr =
      let
        allWorkloads = wlIndex.realms.workloads.all;
        allNames = map (w: w.workloadName) allWorkloads;
      in {
        containsArchived = builtins.elem "archived" allNames;
        containsCorpLaptop = builtins.elem "corp-laptop" allNames;
      };
    expected = {
      containsArchived = true;
      containsCorpLaptop = true;
    };
  };

  # ── workload index: byVm accessor ────────────────────────────────────────────
  "realm-workloads/index-by-vm-accessor" = {
    expr =
      let
        byVm = wlIndex.realms.workloads.byVm;
        corpboxWorkloads = byVm.corpbox or [ ];
        enabledCorpboxNames = map (w: w.workloadName)
          (lib.filter (w: w.enable) corpboxWorkloads);
      in {
        corpboxHasWorkloads = corpboxWorkloads != [ ];
        corpLaptopPresent = builtins.elem "corp-laptop" enabledCorpboxNames;
        # archived is disabled → still shows in byVm (byVm tracks enabled rows)
        # per implementation: byVm uses enabledRealmWorkloadRows (enable = true only)
        archivedAbsent = !(builtins.elem "archived" (map (w: w.workloadName) corpboxWorkloads));
        nullRefNotInByVm = !(byVm ? null);
      };
    expected = {
      corpboxHasWorkloads = true;
      corpLaptopPresent = true;
      archivedAbsent = true;
      nullRefNotInByVm = true;
    };
  };

  # ── workload index: realm row workloadNames convenience field ────────────────
  "realm-workloads/realm-row-workload-names" = {
    expr = {
      allNames = workRealm.workloadNames;
      enabledNames = workRealm.enabledWorkloadNames;
    };
    expected = {
      # sorted alphabetically by name (sortedMapAttrsToList)
      allNames = [ "archived" "corp-laptop" "provider-service" ];
      enabledNames = [ "corp-laptop" "provider-service" ];
    };
  };

  # ── launcher JSON: shape and required fields ─────────────────────────────────
  "realm-workloads/launcher-json-shape" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data;
        corpRow = lib.findFirst
          (w: w.workloadName == "corp-laptop")
          null
          data.workloads;
        providerRow = lib.findFirst
          (w: w.workloadName == "provider-service")
          null
          data.workloads;
      in {
        schemaVersion = data.schemaVersion;
        runtimeState = data.runtimeState;
        workloadCount = builtins.length data.workloads;
        corpFields = {
          realmName = corpRow.realmName;
          realmPath = corpRow.realmPath;
          workloadName = corpRow.workloadName;
          targetAddress = corpRow.targetAddress;
          actionId = corpRow.actionId;
          label = corpRow.label;
          icon = corpRow.icon;
          capabilityRefs = corpRow.capabilityRefs;
          legacyVmName = corpRow.legacyVmName;
          substrateId = corpRow.substrateId;
          runtimeKind = corpRow.runtimeKind;
          runtimeProviderId = corpRow.runtimeProviderId;
          vsockCidIsInt = builtins.isInt corpRow.vsockCid;
        };
        providerFields = {
          workloadName = providerRow.workloadName;
          legacyVmName = providerRow.legacyVmName;
          runtimeKind = providerRow.runtimeKind;
          vsockCid = providerRow.vsockCid;
        };
      };
    expected = {
      schemaVersion = "v1";
      runtimeState = "metadata-only";
      # only enabled workloads appear in the launcher JSON
      workloadCount = 2;
      corpFields = {
        realmName = "work";
        realmPath = "work.home";
        workloadName = "corp-laptop";
        targetAddress = "corp-laptop.work.home.d2b";
        actionId = "corp-laptop";
        label = "Corp Laptop";
        icon = "computer-laptop";
        capabilityRefs = [ "graphics" "guest-exec" ];
        legacyVmName = "corpbox";
        substrateId = "corpbox";
        runtimeKind = "nixos";
        runtimeProviderId = "local-cloud-hypervisor";
        vsockCidIsInt = true;
      };
      providerFields = {
        workloadName = "provider-service";
        legacyVmName = null;
        runtimeKind = null;
        # no legacyVmName → vsockCid must be null
        vsockCid = null;
      };
    };
  };

  # ── launcher JSON: invariants block ─────────────────────────────────────────
  "realm-workloads/launcher-json-invariants" = {
    expr = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data.invariants;
    expected = {
      noSecretsOrCredentials = true;
      noCommandPayloads = true;
      noOpaqueSessionHandles = true;
      noProviderTokens = true;
      metadataOnly = true;
    };
  };

  # ── launcher JSON: bundle artifact registration ───────────────────────────────
  "realm-workloads/launcher-json-bundle-artifact" = {
    expr =
      let
        artifact = wlCfg.d2b._bundle.realmWorkloadsLauncherJson;
      in {
        installFileName = artifact.installFileName;
        classification = artifact.classification;
        sensitivity = artifact.sensitivity;
        installedAtEtc = wlCfg.environment.etc ? "d2b/realm-workloads-launcher.json";
      };
    expected = {
      installFileName = "realm-workloads-launcher.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      installedAtEtc = true;
    };
  };

  # ── launcher JSON: disabled workload excluded from emitted JSON ───────────────
  "realm-workloads/launcher-json-excludes-disabled" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data;
        names = map (w: w.workloadName) data.workloads;
      in {
        archivedExcluded = !(builtins.elem "archived" names);
        corpPresent = builtins.elem "corp-laptop" names;
      };
    expected = {
      archivedExcluded = true;
      corpPresent = true;
    };
  };

  # ── launcher JSON: duplicate icon/label — both rows preserved ─────────────────
  # Two workloads in different realms sharing the same icon and label must both
  # appear in the emitted workloads list (no silent deduplication).
  "realm-workloads/launcher-json-no-implicit-dedup" = {
    expr =
      let
        dupFixture = lib.recursiveUpdate hostBase {
          d2b.realms.realm-a = {
            path = "realm-a";
            env = "home";
            network.envs = [ "home" ];
            workloads.browser = {
              launcher.label = "Web Browser";
              launcher.icon.id = "web-browser";
            };
          };
          d2b.realms.realm-b = {
            path = "realm-b";
            env = "dev";
            network.envs = [ "dev" ];
            workloads.browser = {
              launcher.label = "Web Browser";
              launcher.icon.id = "web-browser";
            };
          };
        };
        data = (mkEval [ dupFixture ]).config.d2b._bundle.realmWorkloadsLauncherJson.data;
        browserRows = lib.filter (w: w.workloadName == "browser") data.workloads;
      in {
        bothPresent = builtins.length browserRows == 2;
        distinctRealms = lib.sort lib.lessThan
          (lib.unique (map (w: w.realmPath) browserRows));
      };
    expected = {
      bothPresent = true;
      distinctRealms = [ "realm-a" "realm-b" ];
    };
  };

  # ── cross-realm vsock CID collision: assertion fires ─────────────────────────
  # svc2532 and svc4319 are envless VMs with the same hash-based CID.
  # Workloads in different realms reference them → cross-realm assertion fires.
  "realm-workloads/cross-realm-cid-collision-assertion-fires" = {
    expr = hasMessage [
      "Cross-realm vsock CID collision"
      "alpha/wl-a"
      "beta/wl-b"
    ] cidCollisionMessages;
    expected = true;
  };

  # ── cross-realm vsock CID collision: message names affected workloads ─────────
  "realm-workloads/cross-realm-cid-collision-names-affected-pairs" = {
    expr = hasMessage [
      "Cross-realm vsock CID collision"
      "svc2532"
      "svc4319"
    ] cidCollisionMessages;
    expected = true;
  };

  # ── cross-realm vsock CID: same VM in two realms — assertion does NOT fire ────
  # When both workloads reference the SAME VM, the cross-realm assertion must
  # not fire (same-VM CID sharing is intentional).
  "realm-workloads/cross-realm-same-vm-no-cid-collision" = {
    expr =
      let
        crossRealmMessages = lib.filter
          (msg: lib.hasInfix "Cross-realm vsock CID collision" msg)
          sameVmMessages;
      in {
        noCollisionFired = crossRealmMessages == [ ];
        configEvalsClean = lib.all (a: a.assertion) sameVmCfg.assertions;
      };
    expected = {
      noCollisionFired = true;
      configEvalsClean = true;
    };
  };

  # ── cross-realm external-network conflict: index populated ───────────────────
  # Two realms linking to the same env (which has attachment enabled) are
  # detected as conflicting; the index exposes the conflict record.
  "realm-workloads/cross-realm-ext-network-conflict-in-index" = {
    expr =
      let
        conflicts = extNetCfg.d2b._index.realms.externalNetworkConflicts;
        first = builtins.head conflicts;
      in {
        conflictsNonEmpty = conflicts != [ ];
        interface = first.interface;
        bothRealmsPresent =
          builtins.elem "realm-a" first.realmNames &&
          builtins.elem "realm-b" first.realmNames;
        envPresent = builtins.elem "work" first.envNames;
      };
    expected = {
      conflictsNonEmpty = true;
      interface = "eth0";
      bothRealmsPresent = true;
      envPresent = true;
    };
  };

  # ── cross-realm external-network conflict: advisory (assertion = true) ────────
  # The assertion is explicitly advisory in metadata-only runtime state: it
  # MUST NOT fail eval (assertion = true means "passes").
  "realm-workloads/cross-realm-ext-network-conflict-advisory-only" = {
    expr =
      let
        conflictMsgInFailed = lib.any
          (msg: lib.hasInfix "externalNetwork.attachment.interface" msg)
          extNetMessages;
      in {
        assertionDoesNotFail = !conflictMsgInFailed;
        configEvalsClean = lib.all (a: a.assertion) extNetCfg.assertions;
      };
    expected = {
      assertionDoesNotFail = true;
      configEvalsClean = true;
    };
  };

  # ── realm with no workloads: index exposes empty lists ────────────────────────
  "realm-workloads/realm-with-no-workloads-empty-index" = {
    expr =
      let
        homeRealm = wlIndex.realms.byPath.home or null;
      in {
        workloadsEmpty = homeRealm.workloads == [ ];
        workloadNamesEmpty = homeRealm.workloadNames == [ ];
        enabledWorkloadNamesEmpty = homeRealm.enabledWorkloadNames == [ ];
      };
    expected = {
      workloadsEmpty = true;
      workloadNamesEmpty = true;
      enabledWorkloadNamesEmpty = true;
    };
  };
}
