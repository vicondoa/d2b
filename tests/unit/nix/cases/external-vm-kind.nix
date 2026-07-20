# Foundational external VM runtime coverage for the qemu-media runtime
# provider.
#
# ADR 0045 replaced the legacy `d2b.vms.<name>.runtime.kind` /
# `d2b.envs` / `d2b.manifest` model with the realm/workload/provider
# contract: a `providers.<name>.implementationId = "qemu-media"` runtime
# provider is bound to a workload via `providerRefs.runtime`, and
# `nixos-modules/index-resources.nix` derives the opaque roles/resources
# that `processes-json.nix`, `provider-registry-v2-json.nix`, and
# `minijail-profiles.nix` render. These cases pin that contract: the
# qemu-media role set (and the cloud-hypervisor-only roles it must NOT
# gain), the rendered qemu-media runner process/minijail shape, the
# opaque host-json/provider-registry output, the absence of raw
# identities/process markers in the rendered artifacts, and the
# assertions/omissions that reject incompatible or under-specified
# capability combinations.
{ mkEval, lib, flakeRoot, system, ... }:

let
  mkHost = { workloadExtra ? { }, providerExtra ? { } }:
    { lib, ... }: {
      boot.loader.grub.enable = false;
      boot.loader.systemd-boot.enable = false;
      boot.initrd.includeDefaultModules = false;
      fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
      environment.etc."machine-id".text = "00000000000000000000000000000000";
      system.stateVersion = "25.11";
      d2b.acceptDestructiveV2Cutover = true;

      users.users.alice = { isNormalUser = true; uid = 1000; };

      d2b.site = {
        waylandUser = "alice";
        launcherUsers = [ "alice" ];
        yubikey.enable = false;
      };

      d2b.realms.local-root = {
        path = "local-root";
        placement = "host-local";
      };
      d2b.realms.kiosk = {
        parent = "local-root";
        path = "kiosk.local-root";
        placement = "host-local";
        allowedUsers = [ "alice" ];
        broker = {
          enable = true;
          hostMutation = true;
        };
        network = {
          mode = "declared";
          lanSubnet = "10.70.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };
        providers.qemu = {
          type = "runtime";
          implementationId = "qemu-media";
          configRef = "kiosk-player-config";
          capabilities = [ "qmp-media-attach" ];
        } // providerExtra;
        workloads.player = {
          providerRefs.runtime = "qemu";
          autostart = false;
        } // workloadExtra;
      };
    };

  positive = mkEval [ (mkHost { }) ];
  cfg = positive.config;
  index = cfg.d2b._index;
  workload = lib.findFirst
    (row: row.workloadName == "player")
    (throw "normalized player workload missing")
    index.workloads.enabledList;
  localRoot = index.realms.byName.local-root;
  runtimeBinding = workload.providerBindings.runtime;
  runtimeProvider = index.providers.byId.${runtimeBinding.providerId};
  roles = index.roles.byWorkloadId.${workload.workloadId};
  roleKinds = map (row: row.roleKind) roles;
  qemuRole = lib.findFirst
    (row: row.roleKind == "qemu-media")
    (throw "normalized player qemu-media role missing")
    roles;

  processes = cfg.d2b._bundle.processesJson.data.vms;
  workloadProcess = lib.findFirst
    (row: row.vm == workload.workloadId)
    (throw "rendered player process DAG missing")
    processes;
  qemuNode = lib.findFirst
    (row: row.id == qemuRole.roleId)
    (throw "rendered player qemu-media node missing")
    workloadProcess.nodes;

  minijailProfile =
    cfg.d2b._bundle.minijailProfiles."role-${qemuRole.roleId}".data;

  hostJson = cfg.d2b._bundle.hostJson.data;
  providerRegistry = cfg.d2b._bundle.providerRegistryV2Json.data.providers;
  runtimeRegistry = lib.findFirst
    (row:
      (row.binding.axis or null) == "local-runtime"
      && (row.binding.workloadId or null) == workload.workloadId)
    (throw "player runtime registry binding missing")
    providerRegistry;

  rawArtifactText = builtins.toJSON {
    host = hostJson;
    process = workloadProcess;
    registry = providerRegistry;
    minijail = minijailProfile;
    normalizedResources = map
      (row: {
        inherit (row) kind path providerId realmId resourceId roleId workloadId;
      })
      index.resources.byWorkloadId.${workload.workloadId};
  };

  # A runtime provider whose `implementationId` is unsupported by both
  # `localVmRoles` and `qemuRoles` (see `index-resources.nix`'s
  # `rolesFor`). Nothing throws or asserts here; the workload is simply
  # excluded from the rendered process DAG (`workload-process-rows.nix`
  # filters on `builtins.elem runtime ["cloud-hypervisor" "qemu-media"]`)
  # -- the fail-closed-by-omission analogue of the removed "rejects
  # unsupported runtime kind" assertion.
  unsupportedRuntime = mkEval [
    (mkHost { providerExtra = { implementationId = "bhyve"; }; })
  ];
  unsupportedCfg = unsupportedRuntime.config;
  unsupportedWorkload = builtins.head unsupportedCfg.d2b._index.workloads.enabledList;
  unsupportedRoles = map
    (row: row.roleKind)
    unsupportedCfg.d2b._index.roles.byWorkloadId.${unsupportedWorkload.workloadId};
  unsupportedProcessRows = builtins.filter
    (row: row.vm == unsupportedWorkload.workloadId)
    unsupportedCfg.d2b._bundle.processesJson.data.vms;

  # Device-gated features (tpm/graphics/usbip/security-key) all share the
  # same realm/workload "explicit device provider binding" assertion pair;
  # exercise all four to keep the coverage of the removed per-feature
  # `rejects-tpm` / `rejects-graphics` / `rejects-usbip` cases.
  deviceFeatureMessages = feature:
    let
      unbound = mkEval [ (mkHost { workloadExtra = { ${feature}.enable = true; }; }) ];
    in
    map (a: a.message)
      (builtins.filter (a: !a.assertion) unbound.config.assertions);
  deviceFeatureRejections =
    map (feature: { inherit feature; messages = deviceFeatureMessages feature; })
      [ "tpm" "graphics" "usbip" "securityKey" ];
  deviceBindingMessages = [
    "d2b realm device resources require exactly one host-mediated device provider in the workload realm."
    "Workload player.kiosk.local-root.d2b: TPM, graphics, USBIP, and security-key features require an explicit device provider binding."
  ];
  # `graphics` also trips the x86_64-linux-only platform gate
  # (`assertions.nix`'s `graphics/audio components are supported only on
  # x86_64-linux`) on any other system, alongside the device-binding
  # rejection every feature shares.
  expectedDeviceFeatureMessagesFor = feature:
    if feature == "graphics" && system != "x86_64-linux"
    then
      [ (builtins.elemAt deviceBindingMessages 0) ]
      ++ [ "Workload player.kiosk.local-root.d2b: graphics/audio components are supported only on x86_64-linux." ]
      ++ [ (builtins.elemAt deviceBindingMessages 1) ]
    else deviceBindingMessages;

  # `audio.enable`/`display.wayland` without a matching provider binding
  # throw during normalization rather than surfacing as a config
  # assertion (Bucket B, mirrors `assertions.nix`'s `expectedError`
  # convention).
  audioWithoutBinding =
    (mkEval [ (mkHost { workloadExtra.audio.enable = true; }) ]).config.assertions;
  displayWithoutBinding =
    (mkEval [ (mkHost { workloadExtra.display.wayland = true; }) ]).config.assertions;

  undeclaredRuntimeProvider =
    mkEval [ (mkHost { workloadExtra.providerRefs.runtime = "does-not-exist"; }) ];
  undeclaredRuntimeMessages =
    map (a: a.message)
      (builtins.filter (a: !a.assertion) undeclaredRuntimeProvider.config.assertions);

  # The legacy `d2b.vms.<name>` schema exposed `ssh`, `sudo`,
  # `homeManager`, `audit`, `observability`, and `guest.control` /
  # `guestConfigFile` options for per-VM guest configuration. None of
  # these exist on `d2b.realms.<realm>.workloads.<name>` any more; the
  # module system rejects them outright (option-does-not-exist), which is
  # the current-tree analogue of the removed
  # `rejects-guest-control` / `rejects-ssh-sudo-keys` /
  # `rejects-home-manager` / `rejects-audit` / `rejects-observability`
  # cases.
  legacyGuestFieldEvals = map
    (extra: (mkEval [ (mkHost { workloadExtra = extra; }) ]).config.assertions)
    [
      { ssh.user = "alice"; }
      { sudo.enable = true; }
      { homeManager.enable = true; }
      { audit.enable = true; }
      { observability.enable = true; }
      { guest.control.enable = true; }
      { guestConfigFile = flakeRoot + "/tests/unit/nix/eval-cases/guest-fixtures/clean-guest.nix"; }
    ];
in
{
  "external-vm-kind/evaluates-without-hardware" = {
    expr = {
      assertionsGreen = lib.all (assertion: assertion.assertion) cfg.assertions;
      platformBinary =
        lib.hasSuffix
          (if system == "x86_64-linux"
           then "/bin/qemu-system-x86_64"
           else "/bin/qemu-system-aarch64")
          qemuNode.binaryPath;
    };
    expected = {
      assertionsGreen = true;
      platformBinary = true;
    };
  };

  "external-vm-kind/qemu-media-role-set-excludes-cloud-hypervisor-roles" = {
    expr = {
      inherit roleKinds;
      hasCloudHypervisorOnlyRoles =
        lib.any
          (kind: builtins.elem kind
            [ "cloud-hypervisor" "virtiofsd" "guest-control-health" "store-virtiofs-preflight" ])
          roleKinds;
    };
    expected = {
      roleKinds = [ "qemu-media" "vsock-relay" ];
      hasCloudHypervisorOnlyRoles = false;
    };
  };

  "external-vm-kind/runtime-provider-binding-is-opaque" = {
    expr = {
      inherit (runtimeBinding) implementationId providerType;
      runtimeProviderIsOpaqueId = runtimeBinding.providerId == runtimeProvider.providerId;
      provider = {
        inherit (runtimeProvider) capabilityRefs configRef implementationId providerType;
      };
    };
    expected = {
      implementationId = "qemu-media";
      providerType = "runtime";
      runtimeProviderIsOpaqueId = true;
      provider = {
        capabilityRefs = [ "qmp-media-attach" ];
        configRef = "kiosk-player-config";
        implementationId = "qemu-media";
        providerType = "runtime";
      };
    };
  };

  "external-vm-kind/qemu-media-runner-process-contract" = {
    expr = {
      role = qemuNode.role;
      startsPaused = builtins.elem "-S" qemuNode.argv;
      hasGtkDisplay =
        builtins.elem "-display" qemuNode.argv
        && builtins.elem "gtk,gl=off,show-cursor=on" qemuNode.argv;
      tapUsesVhostOff = lib.any (arg: lib.hasInfix "vhost=off" arg) qemuNode.argv;
      noVhostNetDeviceLiteral = !(lib.any (arg: lib.hasInfix "/dev/vhost-net" arg) qemuNode.argv);
      noRawWorkloadOrRealmName =
        !(lib.any (arg: lib.hasInfix "player" arg) qemuNode.argv)
        && !(lib.any (arg: lib.hasInfix "kiosk" arg) qemuNode.argv);
      readiness = qemuNode.readiness;
    };
    expected = {
      role = "qemu-media-runner";
      startsPaused = true;
      hasGtkDisplay = true;
      tapUsesVhostOff = true;
      noVhostNetDeviceLiteral = true;
      noRawWorkloadOrRealmName = true;
      readiness = [
        {
          kind = "unix-socket-listening";
          value =
            "/run/d2b/r/${workload.realmId}/w/${workload.workloadId}/roles/${qemuRole.roleId}/qmp.sock";
        }
      ];
    };
  };

  "external-vm-kind/qemu-media-minijail-profile" = {
    expr = {
      inherit (minijailProfile) capabilities principal profileId role seccompPolicyRef;
      hasKvmBind = builtins.elem "/dev/kvm" minijailProfile.mountPolicy.deviceBinds;
      noUserNamespace = minijailProfile.userNamespace == null && !minijailProfile.namespaces.user;
      noNetNamespace = !minijailProfile.namespaces.net;
    };
    expected = {
      capabilities = [ ];
      principal = "d2b-role-${qemuRole.roleId}";
      profileId = "role-${qemuRole.roleId}";
      role = "qemu-media-runner";
      seccompPolicyRef = "w1-qemu-media";
      hasKvmBind = true;
      noUserNamespace = true;
      noNetNamespace = true;
    };
  };

  "external-vm-kind/host-json-has-only-opaque-media" = {
    expr = {
      inherit (hostJson) qemuMedia runtimeProviders vmRuntimes;
      workloadIfName = lib.findFirst
        (row: (row.vm or null) == workload.workloadId)
        null
        hostJson.ifNameMappings;
    };
    expected = {
      qemuMedia = null;
      runtimeProviders = [ ];
      vmRuntimes = [ ];
      workloadIfName = {
        derivedIfname = "d2b-t93CA74BA";
        env = workload.realmId;
        role = "workload-lan";
        userVisibleName = "d2b-t93CA74BA";
        vm = workload.workloadId;
      };
    };
  };

  "external-vm-kind/provider-registry-runtime-authority-is-external" = {
    expr = {
      inherit (runtimeRegistry.descriptor) implementationId;
      inherit (runtimeRegistry.descriptor.authority) type;
      posture = runtimeRegistry.descriptor.authority.posture;
      binding = {
        inherit (runtimeRegistry.binding) runnerIntentId vmStartIntentId workloadId;
      };
    };
    expected = {
      implementationId = "qemu-media";
      type = "runtime";
      posture = {
        cgroup = "realm-delegated-leaf";
        deviceMediation = "broker-delegated-typed";
        network = "isolated-namespace";
        persistentIdentity = "none";
        process = "provider-owned-pidfd";
        userNamespace = "none";
      };
      binding = {
        runnerIntentId = "runner:workload:${workload.workloadId}:role:${qemuRole.roleId}";
        vmStartIntentId = "vm-start:workload:${workload.workloadId}:role:${qemuRole.roleId}";
        workloadId = workload.workloadId;
      };
    };
  };

  # Argv/socket paths are opaque short IDs; only the correlation-oriented
  # `workloadIdentity` block legitimately carries the configured
  # realm/workload names (already covered by the argv assertions above), so
  # this case -- like `requested-vm-config/no-raw-usb-identities-in-artifacts`
  # -- scopes "no raw identities" to physical/hardware device identifiers,
  # which the qemu-media runner and its provider/registry rows must never
  # surface.
  "external-vm-kind/no-raw-hardware-identities-in-artifacts" = {
    expr =
      !(lib.hasInfix "/dev/disk/by-id" rawArtifactText)
      && !(lib.hasInfix "/dev/bus/usb" rawArtifactText)
      && !(lib.hasInfix "usbSelector" rawArtifactText)
      && !(lib.hasInfix "busid" rawArtifactText)
      && !(lib.hasInfix "busId" rawArtifactText)
      && !(lib.hasInfix "SecretSerial" rawArtifactText)
      && !(lib.hasInfix "1-2.3" rawArtifactText);
    expected = true;
  };

  "external-vm-kind/no-process-marker-sentinels-in-artifacts" = {
    expr =
      !(lib.hasInfix "ForbiddenLiveOSName" rawArtifactText)
      && !(lib.hasInfix "Windows" rawArtifactText)
      && !(lib.hasInfix "macOS" rawArtifactText)
      && !(lib.hasInfix "( W" rawArtifactText)
      && !(lib.hasInfix "W3fu" rawArtifactText)
      && !(lib.hasInfix "P6" rawArtifactText);
    expected = true;
  };

  "external-vm-kind/rejects-unsupported-runtime-implementation-by-omission" = {
    expr = {
      assertionsGreen = lib.all (assertion: assertion.assertion) unsupportedCfg.assertions;
      roleKinds = unsupportedRoles;
      processRowCount = builtins.length unsupportedProcessRows;
    };
    expected = {
      assertionsGreen = true;
      roleKinds = [ "vsock-relay" ];
      processRowCount = 0;
    };
  };

  "external-vm-kind/rejects-device-features-without-binding" = {
    expr = lib.all
      (row: row.messages == expectedDeviceFeatureMessagesFor row.feature)
      deviceFeatureRejections;
    expected = true;
  };

  "external-vm-kind/rejects-audio-without-binding" = {
    expr = audioWithoutBinding;
    expectedError = { };
  };

  "external-vm-kind/rejects-display-without-binding" = {
    expr = displayWithoutBinding;
    expectedError = { };
  };

  "external-vm-kind/rejects-undeclared-runtime-provider-ref" = {
    expr =
      lib.any (lib.hasInfix "must name an enabled runtime provider") undeclaredRuntimeMessages
      && lib.any (lib.hasInfix "selects undeclared runtime provider does-not-exist") undeclaredRuntimeMessages;
    expected = true;
  };

  "external-vm-kind/rejects-legacy-guest-config-fields" = {
    expr = map (assertions: builtins.deepSeq assertions true) legacyGuestFieldEvals;
    expectedError = { };
  };
}
