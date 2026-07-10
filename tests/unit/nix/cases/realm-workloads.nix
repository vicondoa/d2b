# nix-unit coverage for realm-owned workload index, launcher metadata, and
# cross-realm assertion contracts introduced in Wave 14, extended in Wave 15.
#
# Coverage:
#   • Accepted workload config shapes (legacyVmName present, provider-placeholder, disabled)
#   • Workload index row rendering: targetAddress, canonicalTarget, runtimeKind,
#     substrateId, capabilityRefs sorted+deduped, appCommand, actions,
#     all/enabled/byVm accessors
#   • realm-workloads-launcher.json emitter: schemaVersion, runtimeState,
#     per-workload fields (incl. canonicalTarget, appCommand, actions),
#     vsockCid advisory, invariants block (noSensitiveCommandPayloads)
#   • Bundle artifact registration: installFileName, classification,
#     sensitivity, /etc install mode
#   • Cross-realm vsock CID collision assertion: fires when two workloads in
#     different realms reference different VMs with the same derived CID;
#     same-VM cross-realm references do NOT trigger the assertion
#   • Cross-realm external-network attachment conflict: advisory (assertion
#     stays true) but index.realms.externalNetworkConflicts is populated
#   • controller config: explicit workload identity is nested under `identity`
#     with correct WorkloadIdentity DTO field names (workloadId, realmId,
#     realmPath as array, canonicalTarget, providerId); kind and runtimeProviderId
#     are absent at the workload root
#   • launcher canonicalTarget override uses a valid .d2b target address
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
  # One realm ("work.home") with two workloads: one with a legacyVmName, one without.
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
          app.command = "d2b vm exec corp-laptop -- bash -l";
          actions = [
            { id = "open-terminal"; label = "Open Terminal"; command = "d2b vm exec corp-laptop -- bash -l"; }
            { id = "restart"; label = "Restart"; command = "d2b vm restart corp-laptop"; }
          ];
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

  unsafeLocalFixture = lib.recursiveUpdate hostBase {
    d2b.realms.host = {
      allowedUsers = [ "alice" ];
      policy.allowUnsafeLocal = true;
      network.ui.accentColor = "#cc3344";
      workloads.tools = {
        kind = "unsafe-local";
        shell = {
          enable = true;
          defaultName = "host";
          maxSessions = 8;
        };
        launcher = {
          enable = true;
          label = "Local tools";
          icon.name = "applications-system";
          defaultItem = "browser";
          items = {
            browser = {
              type = "exec";
              name = "Firefox";
              icon.name = "firefox";
              argv = [ "firefox" "private-canary-argv" ];
              graphical = true;
            };
            terminal = {
              type = "shell";
              name = "Terminal";
              icon.name = "terminal";
            };
          };
        };
      };
    };
  };
  unsafeCfg = (mkEval [ unsafeLocalFixture ]).config;
  unsafeRow = builtins.head unsafeCfg.d2b._index.realms.workloads.enabled;

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
        # canonicalTarget defaults to targetAddress when launcher.app.targetRealm is null
        canonicalTargetEqualsTargetAddress = row.canonicalTarget == row.targetAddress;
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
        appCommand = row.appCommand;
        actionsCount = builtins.length row.actions;
        firstActionId = (builtins.head row.actions).id;
      };
    expected = {
      workloadName = "corp-laptop";
      realmName = "work";
      realmPath = "work.home";
      targetAddress = "corp-laptop.work.home.d2b";
      canonicalTargetEqualsTargetAddress = true;
      substrateId = "corpbox";
      legacyVmName = "corpbox";
      runtimeKind = "nixos";
      runtimeProviderId = "local-cloud-hypervisor";
      label = "Corp Laptop";
      icon = "computer-laptop";
      actionId = "corp-laptop";
      capabilityRefs = [ "graphics" "guest-exec" ];
      enable = true;
      appCommand = "d2b vm exec corp-laptop -- bash -l";
      actionsCount = 2;
      firstActionId = "open-terminal";
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
          workloadId = corpRow.workloadId;
          targetAddress = corpRow.targetAddress;
          canonicalTarget = corpRow.canonicalTarget;
          actionId = corpRow.actionId;
          label = corpRow.label;
          icon = corpRow.icon;
          iconId = corpRow.iconId;
          iconName = corpRow.iconName;
          iconGroupKey = corpRow.iconGroupKey;
          capabilityRefs = corpRow.capabilityRefs;
          appCommand = corpRow.appCommand;
          actionsCount = builtins.length corpRow.actions;
          firstActionId = (builtins.head corpRow.actions).id;
          firstActionLabel = (builtins.head corpRow.actions).label;
          legacyVmName = corpRow.legacyVmName;
          substrateId = corpRow.substrateId;
          runtimeKind = corpRow.runtimeKind;
          runtimeProviderId = corpRow.runtimeProviderId;
          vsockCidIsInt = builtins.isInt corpRow.vsockCid;
        };
        providerFields = {
          workloadName = providerRow.workloadName;
          workloadId = providerRow.workloadId;
          legacyVmName = providerRow.legacyVmName;
          runtimeKind = providerRow.runtimeKind;
          appCommand = providerRow.appCommand;
          actionsEmpty = providerRow.actions == [ ];
          iconId = providerRow.iconId;
          iconName = providerRow.iconName;
          iconGroupKey = providerRow.iconGroupKey;
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
        workloadId = "corp-laptop";
        targetAddress = "corp-laptop.work.home.d2b";
        # canonicalTarget matches targetAddress when no override is set
        canonicalTarget = "corp-laptop.work.home.d2b";
        actionId = "corp-laptop";
        label = "Corp Laptop";
        icon = "computer-laptop";
        iconId = "computer-laptop";
        iconName = null;
        iconGroupKey = "computer-laptop";
        capabilityRefs = [ "graphics" "guest-exec" ];
        appCommand = "d2b vm exec corp-laptop -- bash -l";
        actionsCount = 2;
        firstActionId = "open-terminal";
        firstActionLabel = "Open Terminal";
        legacyVmName = "corpbox";
        substrateId = "corpbox";
        runtimeKind = "nixos";
        runtimeProviderId = "local-cloud-hypervisor";
        vsockCidIsInt = true;
      };
      providerFields = {
        workloadName = "provider-service";
        workloadId = "provider-service";
        legacyVmName = null;
        runtimeKind = null;
        appCommand = null;
        actionsEmpty = true;
        iconId = null;
        iconName = null;
        iconGroupKey = null;
        # no legacyVmName → vsockCid must be null
        vsockCid = null;
      };
    };
  };

  # ── launcher JSON: canonicalTarget override ──────────────────────────────────
  # When launcher.app.targetRealm is set, canonicalTarget must use the override
  # rather than the derived targetAddress.  The override value must end in
  # `.d2b` to be a valid WorkloadTarget; `corp-laptop.alt.d2b` is a valid
  # target that differs from the default `corp-laptop.work.home.d2b`.
  "realm-workloads/launcher-json-canonical-target-override" = {
    expr =
      let
        overrideFixture = lib.recursiveUpdate workloadFixture {
          d2b.realms.work.workloads.corp-laptop.launcher.app.targetRealm =
            "corp-laptop.alt.d2b";
        };
        data = (mkEval [ overrideFixture ]).config.d2b._bundle.realmWorkloadsLauncherJson.data;
        row = lib.findFirst (w: w.workloadName == "corp-laptop") null data.workloads;
      in {
        targetAddress = row.targetAddress;
        canonicalTarget = row.canonicalTarget;
        overrideDiffers = row.canonicalTarget != row.targetAddress;
      };
    expected = {
      targetAddress = "corp-laptop.work.home.d2b";
      canonicalTarget = "corp-laptop.alt.d2b";
      overrideDiffers = true;
    };
  };

  # ── launcher JSON: invariants block ─────────────────────────────────────────
  "realm-workloads/launcher-json-invariants" = {
    expr = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data.invariants;
    expected = {
      noSecretsOrCredentials = true;
      # appCommand and actions[].command are static operator-declared launch
      # metadata, not sensitive payloads; the invariant reflects this.
      noSensitiveCommandPayloads = true;
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

  # ── launcher JSON: workloadId field equals workloadName ─────────────────────
  # The launcher row must expose `workloadId` as an explicit DTO-named alias
  # for `workloadName`, matching the WorkloadIdentity.workloadId contract used
  # by daemon/broker consumers.
  "realm-workloads/launcher-json-workload-id-field" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data;
        corpRow = lib.findFirst (w: w.workloadName == "corp-laptop") null data.workloads;
        providerRow = lib.findFirst (w: w.workloadName == "provider-service") null data.workloads;
      in {
        corpWorkloadId = corpRow.workloadId;
        corpWorkloadIdEqualsName = corpRow.workloadId == corpRow.workloadName;
        providerWorkloadId = providerRow.workloadId;
        providerWorkloadIdEqualsName = providerRow.workloadId == providerRow.workloadName;
      };
    expected = {
      corpWorkloadId = "corp-laptop";
      corpWorkloadIdEqualsName = true;
      providerWorkloadId = "provider-service";
      providerWorkloadIdEqualsName = true;
    };
  };

  # ── launcher JSON: iconId and iconName fields present separately ─────────────
  # The launcher row must expose `iconId` (raw launcher.icon.id) and `iconName`
  # (raw launcher.icon.name) in addition to the resolved `icon` string, so that
  # desktop tooling can round-trip option values and distinguish primary-id from
  # symbolic-name.
  "realm-workloads/launcher-json-icon-id-name-fields" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data;
        corpRow = lib.findFirst (w: w.workloadName == "corp-laptop") null data.workloads;
        providerRow = lib.findFirst (w: w.workloadName == "provider-service") null data.workloads;
      in {
        # corp-laptop sets icon.id = "computer-laptop" and no icon.name
        corpIconId = corpRow.iconId;
        corpIconName = corpRow.iconName;
        corpIconResolved = corpRow.icon;
        corpIconIdEqualsResolved = corpRow.iconId == corpRow.icon;
        # provider-service sets neither icon.id nor icon.name
        providerIconId = providerRow.iconId;
        providerIconName = providerRow.iconName;
        providerIconResolved = providerRow.icon;
      };
    expected = {
      corpIconId = "computer-laptop";
      corpIconName = null;
      corpIconResolved = "computer-laptop";
      corpIconIdEqualsResolved = true;
      providerIconId = null;
      providerIconName = null;
      providerIconResolved = null;
    };
  };

  # ── launcher JSON: iconGroupKey stable grouping key ──────────────────────────
  # iconGroupKey must equal iconId when set, else iconName, else null.
  # It is always identical to the resolved `icon` field.
  "realm-workloads/launcher-json-icon-group-key" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmWorkloadsLauncherJson.data;
        corpRow = lib.findFirst (w: w.workloadName == "corp-laptop") null data.workloads;
        providerRow = lib.findFirst (w: w.workloadName == "provider-service") null data.workloads;
      in {
        # corp-laptop: iconGroupKey = iconId (preferred over iconName)
        corpGroupKey = corpRow.iconGroupKey;
        corpGroupKeyEqualsIcon = corpRow.iconGroupKey == corpRow.icon;
        corpGroupKeyEqualsIconId = corpRow.iconGroupKey == corpRow.iconId;
        # provider-service: neither id nor name → null group key
        providerGroupKey = providerRow.iconGroupKey;
      };
    expected = {
      corpGroupKey = "computer-laptop";
      corpGroupKeyEqualsIcon = true;
      corpGroupKeyEqualsIconId = true;
      providerGroupKey = null;
    };
  };

  # ── launcher JSON: iconGroupKey prefers iconId over iconName ─────────────────
  # When both icon.id and icon.name are set, iconGroupKey must equal iconId.
  "realm-workloads/launcher-json-icon-group-key-prefers-id-over-name" = {
    expr =
      let
        bothIconFixture = lib.recursiveUpdate hostBase {
          d2b.realms.home = {
            name = "Home";
            path = "home";
            network.envs = [ "home" ];
            workloads.notes = {
              launcher.label = "Notes";
              launcher.icon.id = "notes-app";
              launcher.icon.name = "notes";
            };
          };
        };
        data = (mkEval [ bothIconFixture ]).config.d2b._bundle.realmWorkloadsLauncherJson.data;
        row = lib.findFirst (w: w.workloadName == "notes") null data.workloads;
      in {
        iconId = row.iconId;
        iconName = row.iconName;
        iconGroupKey = row.iconGroupKey;
        iconResolved = row.icon;
        groupKeyEqualsId = row.iconGroupKey == row.iconId;
      };
    expected = {
      iconId = "notes-app";
      iconName = "notes";
      iconGroupKey = "notes-app";
      iconResolved = "notes-app";
      groupKeyEqualsId = true;
    };
  };

  # ── launcher JSON: iconGroupKey equals iconName when only name is set ─────────
  # When icon.name is set but icon.id is null, iconGroupKey must equal iconName.
  "realm-workloads/launcher-json-icon-group-key-falls-back-to-name" = {
    expr =
      let
        nameOnlyFixture = lib.recursiveUpdate hostBase {
          d2b.realms.home = {
            name = "Home";
            path = "home";
            network.envs = [ "home" ];
            workloads.legacy-app = {
              launcher.label = "Legacy App";
              launcher.icon.name = "application-x-generic";
            };
          };
        };
        data = (mkEval [ nameOnlyFixture ]).config.d2b._bundle.realmWorkloadsLauncherJson.data;
        row = lib.findFirst (w: w.workloadName == "legacy-app") null data.workloads;
      in {
        iconId = row.iconId;
        iconName = row.iconName;
        iconGroupKey = row.iconGroupKey;
        iconResolved = row.icon;
        groupKeyEqualsName = row.iconGroupKey == row.iconName;
      };
    expected = {
      iconId = null;
      iconName = "application-x-generic";
      iconGroupKey = "application-x-generic";
      iconResolved = "application-x-generic";
      groupKeyEqualsName = true;
    };
  };

  # ── launcher JSON: duplicate icon — iconGroupKey identical across realms ───────
  # Two workloads in different realms with the same icon.id must have identical
  # iconGroupKey values so desktop consumers can use it as a cluster key for
  # duplicate-app chooser semantics.
  "realm-workloads/launcher-json-duplicate-icon-group-key-matches" = {
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
        groupKeys = lib.unique (map (w: w.iconGroupKey) browserRows);
      in {
        bothPresent = builtins.length browserRows == 2;
        # Both rows must share exactly one iconGroupKey value.
        singleGroupKey = builtins.length groupKeys == 1;
        theGroupKey = builtins.head groupKeys;
        # workloadId must differ (different realms, same workload name).
        distinctRealms = lib.sort lib.lessThan
          (lib.unique (map (w: w.realmPath) browserRows));
      };
    expected = {
      bothPresent = true;
      singleGroupKey = true;
      theGroupKey = "web-browser";
      distinctRealms = [ "realm-a" "realm-b" ];
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

  "realm-workloads/unsafe-local-index-posture-and-items" = {
    expr = {
      kind = unsafeRow.kind;
      providerKind = unsafeRow.providerKind;
      runtimeKind = unsafeRow.runtimeKind;
      runtimeProviderId = unsafeRow.runtimeProviderId;
      stateDir = unsafeRow.stateDir;
      runDir = unsafeRow.runDir;
      posture = unsafeRow.executionPosture;
      defaultItemId = unsafeRow.defaultItemId;
      itemIds = map (item: item.id) unsafeRow.launcherItems;
      itemTypes = map (item: item.type) unsafeRow.launcherItems;
    };
    expected = {
      kind = "unsafe-local";
      providerKind = "unsafe-local";
      runtimeKind = "unsafe-local";
      runtimeProviderId = "unsafe-local";
      stateDir = null;
      runDir = null;
      posture = {
        isolation = "unsafe-local";
        environment = "systemd-user-manager-ambient";
        displayEnvironment = "wayland-proxy-only";
        executionIdentity = "authenticated-requester-uid";
        sessionPersistence = "user-manager-lifetime";
      };
      defaultItemId = "browser";
      itemIds = [ "browser" "terminal" ];
      itemTypes = [ "exec" "shell" ];
    };
  };

  "realm-workloads/launcher-v2-public-metadata-hides-argv" = {
    expr =
      let
        data = unsafeCfg.d2b._bundle.realmWorkloadsLauncherV2Json.data;
        row = builtins.head data.workloads;
        browser = builtins.head row.items;
        encoded = builtins.toJSON data;
      in {
        schemaVersion = data.schemaVersion;
        providerKind = row.providerKind;
        browserType = browser.type;
        browserName = browser.name;
        browserGraphical = browser.graphical;
        hasConfiguredLaunch = builtins.elem "configured-launch" browser.capabilities;
        hasWindowForwarding = builtins.elem "window-forwarding" browser.capabilities;
        realmAccentColor = row.realmAccentColor;
        argvCanaryAbsent = !(lib.hasInfix "private-canary-argv" encoded);
        argvFieldAbsent = !(lib.hasInfix "\"argv\"" encoded);
      };
    expected = {
      schemaVersion = "v2";
      providerKind = "unsafe-local";
      browserType = "exec";
      browserName = "Firefox";
      browserGraphical = true;
      hasConfiguredLaunch = true;
      hasWindowForwarding = true;
      realmAccentColor = "#cc3344";
      argvCanaryAbsent = true;
      argvFieldAbsent = true;
    };
  };

  "realm-workloads/unsafe-local-private-artifact-carries-argv" = {
    expr =
      let
        data = unsafeCfg.d2b._bundle.unsafeLocalWorkloadsJson.data;
        row = builtins.head data.workloads;
        browser = builtins.head row.items;
      in {
        schemaVersion = data.schemaVersion;
        target = row.identity.canonicalTarget;
        legacyVmNameAbsent = !(row.identity ? legacyVmName);
        defaultItemId = row.defaultItemId;
        browserType = browser.type;
        browserArgv = browser.argv;
        shellPolicy = row.shell;
      };
    expected = {
      schemaVersion = "v2";
      target = "tools.host.d2b";
      legacyVmNameAbsent = true;
      defaultItemId = "browser";
      browserType = "exec";
      browserArgv = [ "firefox" "private-canary-argv" ];
      shellPolicy = {
        defaultName = "host";
        maxSessions = 8;
      };
    };
  };

  "realm-workloads/unsafe-local-omitted-from-launcher-v1" = {
    expr =
      unsafeCfg.d2b._bundle.realmWorkloadsLauncherJson.data.workloads == [ ];
    expected = true;
  };

  "realm-workloads/unsafe-local-artifacts-and-bundle-v10" = {
    expr = {
      launcherV2File =
        unsafeCfg.d2b._bundle.realmWorkloadsLauncherV2Json.installFileName;
      launcherV2Class =
        unsafeCfg.d2b._bundle.realmWorkloadsLauncherV2Json.classification;
      unsafeFile =
        unsafeCfg.d2b._bundle.unsafeLocalWorkloadsJson.installFileName;
      unsafeClass =
        unsafeCfg.d2b._bundle.unsafeLocalWorkloadsJson.classification;
      launcherV2Installed =
        unsafeCfg.environment.etc ? "d2b/realm-workloads-launcher-v2.json";
      unsafeInstalled =
        unsafeCfg.environment.etc ? "d2b/unsafe-local-workloads.json";
      bundleVersion = unsafeCfg.d2b._bundle.bundle.data.bundleVersion;
      bundlePath = unsafeCfg.d2b._bundle.bundle.data.unsafeLocalWorkloadsPath;
    };
    expected = {
      launcherV2File = "realm-workloads-launcher-v2.json";
      launcherV2Class = "contractPublic";
      unsafeFile = "unsafe-local-workloads.json";
      unsafeClass = "contractPrivateNonSecret";
      launcherV2Installed = true;
      unsafeInstalled = true;
      bundleVersion = 10;
      bundlePath = "/etc/d2b/unsafe-local-workloads.json";
    };
  };

  "realm-workloads/unsafe-local-requires-explicit-opt-in" = {
    expr = hasMessage
      [ "kind = \"unsafe-local\"" "allowUnsafeLocal is false" ]
      (failureMessages [
        (lib.recursiveUpdate unsafeLocalFixture {
          d2b.realms.host.policy.allowUnsafeLocal = false;
        })
      ]);
    expected = true;
  };

  "realm-workloads/unsafe-local-rejects-net-vm-port-forward" = {
    expr = hasMessage
      [ "workload \"tools\" is unsafe-local" "no guest" "network address" ]
      (failureMessages [
        (lib.recursiveUpdate unsafeLocalFixture {
          d2b.realms.host.network.externalNetwork = {
            attachment = {
              enable = true;
              interface = "eth0";
            };
            portForwards = [{
              protocol = "tcp";
              listenPort = 8443;
              workload = "tools";
              targetPort = 443;
            }];
          };
        })
      ]);
    expected = true;
  };

  "realm-workloads/unsafe-local-rejects-vm-paths-and-options" = {
    expr =
      let
        messages = failureMessages [
          (lib.recursiveUpdate unsafeLocalFixture {
            d2b.realms.host.workloads.tools = {
              legacyVmName = "homebox";
              stateDir = "/var/lib/d2b/vms/homebox";
              runDir = "/run/d2b/vms/homebox";
              localVm.graphics.enable = true;
            };
          })
        ];
      in
      hasMessage [ "must not declare legacyVmName" ] messages
      && hasMessage [ "must not configure localVm" "qemuMedia runtime options" ] messages;
    expected = true;
  };

  "realm-workloads/unsafe-local-rejects-legacy-shell-commands" = {
    expr = hasMessage
      [ "must use typed launcher.items" ]
      (failureMessages [
        (lib.recursiveUpdate unsafeLocalFixture {
          d2b.realms.host.workloads.tools.launcher.app.command =
            "firefox private-canary-argv";
        })
      ]);
    expected = true;
  };

  "realm-workloads/launcher-item-shape-assertions" = {
    expr =
      let
        nulArg = builtins.fromJSON ''"fire\u0000fox"'';
        badExec = failureMessages [
          (lib.recursiveUpdate unsafeLocalFixture {
            d2b.realms.host.workloads.tools.launcher.items.browser.argv = [ ];
          })
        ];
        badNul = failureMessages [
          (lib.recursiveUpdate unsafeLocalFixture {
            d2b.realms.host.workloads.tools.launcher.items.browser.argv = [ nulArg ];
          })
        ];
        badShell = failureMessages [
          (lib.recursiveUpdate unsafeLocalFixture {
            d2b.realms.host.workloads.tools.shell.enable = false;
          })
        ];
        badDefault = failureMessages [
          (lib.recursiveUpdate unsafeLocalFixture {
            d2b.realms.host.workloads.tools.launcher.defaultItem = "missing";
          })
        ];
      in {
        emptyExecRejected =
          hasMessage [ "invalid item" "Exec argv must be non-empty" ] badExec;
        nulArgRejected = hasMessage [ "NUL-free" ] badNul;
        shellWithoutPolicyRejected =
          hasMessage [ "shell launcher item" "shell.enable is" ] badShell;
        missingDefaultRejected =
          hasMessage [ "defaultItem must name" ] badDefault;
      };
    expected = {
      emptyExecRejected = true;
      nulArgRejected = true;
      shellWithoutPolicyRejected = true;
      missingDefaultRejected = true;
    };
  };

  "realm-workloads/persistent-shell-compatibility-item" = {
    expr =
      let
        cfg = (mkEval [
          (lib.recursiveUpdate workloadFixture {
            d2b.realms.work.workloads.corp-laptop.shell.enable = true;
          })
        ]).config;
        row = lib.findFirst
          (workload: workload.workloadName == "corp-laptop")
          null
          cfg.d2b._index.realms.workloads.enabled;
        shellItems = lib.filter (item: item.type == "shell") row.launcherItems;
      in {
        count = builtins.length shellItems;
        id = (builtins.head shellItems).id;
        capabilities = (builtins.head shellItems).capabilityRefs;
      };
    expected = {
      count = 1;
      id = "terminal";
      capabilities = [ "persistent-shell" "pty" ];
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

  # ── controller config: explicit workload entry carries nested identity ─────────
  # When a realm workload has legacyVmName pointing to an enabled VM, the
  # controller config localRuntime.workloads entry must include a nested
  # `identity` object whose fields match WorkloadIdentity (deny_unknown_fields):
  # required workloadId/realmId/realmPath(array)/canonicalTarget; optional
  # legacyVmName/runtimeKind/providerId.  The old flat fields
  # (kind, runtimeProviderId at workload root) are NOT present.
  "realm-workloads/controller-config-explicit-workload-identity" = {
    expr =
      let
        data = wlCfg.d2b._bundle.realmControllersJson.data;
        workController =
          lib.findFirst (row: row.realmPath == "work.home") null data.controllers;
        workLocal = workController.localRuntime;
        corpEntry =
          lib.findFirst (w: w.workloadId == "corp-laptop") null workLocal.workloads;
        corpIdentity = corpEntry.identity;
      in {
        localRuntimePresent = workLocal != null;
        corpEntryPresent = corpEntry != null;
        identityPresent = corpIdentity != null;
        # top-level workload fields (no flat identity fields at root):
        workloadId = corpEntry.workloadId;
        vmName = corpEntry.vmName;
        # nested identity fields must match WorkloadIdentity DTO:
        identityWorkloadId = corpIdentity.workloadId;
        identityRealmId = corpIdentity.realmId;
        identityRealmPath = corpIdentity.realmPath;
        identityCanonicalTarget = corpIdentity.canonicalTarget;
        identityLegacyVmName = corpIdentity.legacyVmName;
        identityRuntimeKind = corpIdentity.runtimeKind;
        identityProviderId = corpIdentity.providerId;
        # deny_unknown_fields guards: kind must not appear at workload root
        # or as an identity key; runtimeProviderId must not appear as a key.
        kindAbsentAtRoot = !(corpEntry ? kind);
        runtimeProviderIdAbsentAtRoot = !(corpEntry ? runtimeProviderId);
        pathsPresent = corpEntry.paths != null;
        runtimePresent = corpEntry.runtime != null;
      };
    expected = {
      localRuntimePresent = true;
      corpEntryPresent = true;
      identityPresent = true;
      workloadId = "corp-laptop";
      vmName = "corpbox";
      identityWorkloadId = "corp-laptop";
      identityRealmId = "work";
      identityRealmPath = [ "work" "home" ];
      identityCanonicalTarget = "corp-laptop.work.home.d2b";
      identityLegacyVmName = "corpbox";
      identityRuntimeKind = "nixos";
      identityProviderId = "local-cloud-hypervisor";
      kindAbsentAtRoot = true;
      runtimeProviderIdAbsentAtRoot = true;
      pathsPresent = true;
      runtimePresent = true;
    };
  };
}
