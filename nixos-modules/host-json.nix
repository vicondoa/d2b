{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  envMeta = cfg._envMeta;
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;
  anyGraphics = builtins.any (vm: vm.graphics.enable) (lib.attrValues enabledVms);
  anyAudio = builtins.any (vm: vm.audio.enable) (lib.attrValues enabledVms);
  anyTpm = builtins.any (vm: vm.tpm.enable) (lib.attrValues enabledVms);
  anyObservability = builtins.any (vm: vm.observability.enable) (lib.attrValues enabledVms);
  anyUsbip = cfg.site.yubikey.enable && builtins.any (vm: vm.usbip.yubikey) (lib.attrValues enabledVms);
  defaultMtu = 1500;

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  moduleRow = module: feature: requirement: gate: sysctls: jailVisibleDevice: {
    inherit module feature requirement gate sysctls jailVisibleDevice;
  };

  capabilityRow = capability: status: devicesOrModules: sidecars: readiness: notes: {
    inherit capability status devicesOrModules sidecars readiness notes;
  };

  fdRow = resource: brokerOperation: recipient: transfer: jailVisibleDevice: notes: {
    inherit resource brokerOperation recipient transfer jailVisibleDevice notes;
  };

  lanEastWestEnabled = m: m.allowEastWest && cfg.site.allowUnsafeEastWest;
  resolvedMtu = m: if m.mtu != null then m.mtu else defaultMtu;
  resolvedMssClamp = m: if m.mssClamp then (resolvedMtu m) - 40 else null;

  workloadTapNames = envName:
    lib.sort lib.lessThan (lib.mapAttrsToList
      (vmName: _: cfg.manifest.${vmName}.tap)
      (lib.filterAttrs (_: vm: vm.enable && vm.env == envName) cfg.vms));

  workloadTapByVm = envName:
    lib.mapAttrsToList
      (vmName: _: { vm = vmName; tap = cfg.manifest.${vmName}.tap; })
      (lib.filterAttrs (_: vm: vm.enable && vm.env == envName) cfg.vms);

  usbipVmNamesForEnv = envName:
    lib.sort lib.lessThan (lib.attrNames (lib.filterAttrs (_: vm: vm.enable && vm.env == envName && vm.usbip.yubikey) cfg.vms));

  usbipVendorProductAllowlist = map (entry: {
    vendor = lib.fromHexString (lib.removePrefix "0x" entry.vendor);
    product = lib.fromHexString (lib.removePrefix "0x" entry.product);
  }) cfg.host.usbip.allowlist;

  usbipBusidLocksForEnv = envName:
    map (vmName:
      {
        vm = vmName;
        lockOwner = "daemon";
        scope = "per-busid";
        busIds = cfg.vms.${vmName}.usbip.busids;
      }
      // lib.optionalAttrs (usbipVendorProductAllowlist != [ ]) {
        vendorProductAllowlist = usbipVendorProductAllowlist;
      }) (usbipVmNamesForEnv envName);

  ipv6SysctlEntry = ifName: {
    inherit ifName;
    disableIpv6 = 1;
    acceptRa = 0;
    autoconf = 0;
    addrGenMode = 1;
    arpIgnore = 1;
  };

  envIfNames = envName: m:
    lib.unique ([
      m.lanBridge
      m.uplinkBridge
      "${envName}-l1"
      "${envName}-u2"
    ] ++ workloadTapNames envName);

  # live systems now have a broker-written canonical runtime
  # ifname map. Pure flake evals cannot see `/var/lib/...`, so they
  # fall back to the legacy hash algorithm; impure live evals prefer the
  # runtime artifact when it exists.
  hostRuntimePath =
    let override = builtins.getEnv "NIXLING_HOST_RUNTIME_PATH";
    in if override != "" then override else "/var/lib/nixling/runtime/host-runtime.json";

  hostRuntimeIfnames =
    if builtins.pathExists hostRuntimePath then
      let runtime = builtins.fromJSON (builtins.readFile hostRuntimePath);
      in if runtime ? ifnames then runtime.ifnames else [ ]
    else [ ];

  derivedIfNameRoleTag = inputRole:
    if inputRole == "br" || inputRole == "up" then "b"
    else if inputRole == "tap" then "t"
    else throw "nixling host.json: unknown derivedIfName role '${inputRole}'";

  derivedIfNameRuntimeRoleTag = inputRole:
    if inputRole == "br" then "nvl"
    else if inputRole == "up" then "upl"
    else if inputRole == "tap" then "wkl"
    else throw "nixling host.json: unknown derivedIfName role '${inputRole}'";

  legacyDerivedIfName = inputRole: envName: vmName:
    let
      vmSuffix = if vmName == null then "" else vmName;
      input = "${inputRole}:${envName}:${vmSuffix}";
      hashLower = builtins.substring 0 8 (builtins.hashString "sha256" input);
      hashUpper = lib.toUpper hashLower;
      tag = derivedIfNameRoleTag inputRole;
    in "nl-${tag}${hashUpper}";

  derivedIfName = inputRole: envName: vmName:
    let
      runtimeRoleTag = derivedIfNameRuntimeRoleTag inputRole;
      runtimeRow = lib.findFirst
        (row:
          row.env == envName
          && (if (if row ? vm then row.vm else null) == null then vmName == null else row.vm == vmName)
          && row.roleTag == runtimeRoleTag)
        null
        hostRuntimeIfnames;
    in if runtimeRow != null then runtimeRow.derivedIfname else legacyDerivedIfName inputRole envName vmName;

  # One IfNameMapping row per managed bridge/TAP. Exposed under
  # `host.json.ifNameMappings` so the broker can re-validate
  # collision-freeness and so `nixling host check` / `status` can
  # surface the user-visible ↔ derived pair.
  envMappings = envName: m:
    let
      lanRow = {
        env = envName;
        role = "net-vm-lan";
        userVisibleName = m.lanBridge;
        derivedIfname = derivedIfName "br" envName null;
      };
      upRow = {
        env = envName;
        role = "uplink";
        userVisibleName = m.uplinkBridge;
        derivedIfname = derivedIfName "up" envName null;
      };
      workloadRows = map
        (entry: {
          env = envName;
          vm = entry.vm;
          role = "workload-lan";
          userVisibleName = entry.tap;
          derivedIfname = derivedIfName "tap" envName entry.vm;
        })
        (workloadTapByVm envName);
    in [ lanRow upRow ] ++ workloadRows;

  allIfNameMappings =
    lib.concatLists (lib.mapAttrsToList envMappings envMeta);

  derivedIfNameList = map (row: row.derivedIfname) allIfNameMappings;

  runtimeProviders = lib.sortOn (provider: provider.provider.id)
    (lib.attrValues nl.runtimeProviderCatalog);

  qemuMediaSourceId = vmName: slotName: source:
    if source.kind == "physical-usb"
    then (if source.ref != null then source.ref else "invalid-missing-ref")
    else "image-${builtins.substring 0 16 (builtins.hashString "sha256" "${vmName}/${slotName}/${if source.path != null then source.path else "missing-path"}")}";

  qemuMediaSourceRow = vmName: slotName: source: ({
    vm = vmName;
    mediaRef = qemuMediaSourceId vmName slotName source;
    slot = slotName;
    sourceKind = source.kind;
    format = source.format;
    readOnly = source.readOnly;
    registryScope =
      if source.kind == "image-file"
      then "direct-config-path"
      else "root-only-runtime-state";
  } // lib.optionalAttrs (source.kind == "image-file") {
    imagePath = source.path;
  });

  qemuMediaSourceRowsForVm = vmName: vm:
    let
      bootRows =
        if vm.qemuMedia.source == null
        then [ ]
        else [ (qemuMediaSourceRow vmName "boot" vm.qemuMedia.source) ];
      slotRows = lib.flatten (lib.mapAttrsToList
        (slotName: slot:
          if slot.source == null
          then [ ]
          else [ (qemuMediaSourceRow vmName slotName slot.source) ])
        vm.qemuMedia.removableSlots);
    in bootRows ++ slotRows;

  qemuMediaSources = lib.sortOn (row: "${row.vm}/${row.mediaRef}/${row.slot}")
    (lib.concatLists (lib.mapAttrsToList qemuMediaSourceRowsForVm (nl.qemuMediaVms cfg.vms)));

  vmRuntimeRows = lib.sortOn (row: row.vm) (lib.mapAttrsToList
    (name: vm:
      let manifest = cfg.manifest.${name};
      in {
        vm = name;
        runtime = nl.vmRuntimeMetadata name vm;
        env = manifest.env;
        stateDir = manifest.stateDir;
        tap = manifest.tap;
        bridge = manifest.bridge;
        staticIp = manifest.staticIp;
        netVm = manifest.netVm;
      })
    enabledVms);

  duplicateDerived =
    lib.unique (lib.filter
      (n: lib.length (lib.filter (m: m == n) derivedIfNameList) > 1)
      derivedIfNameList);

  # Emitter-time collision detection. The broker also
  # re-runs `nixling_host::ifname::detect_collisions` against the
  # trusted bundle copy at runtime; this assert is the first gate.
  ifNameCollisionMessage =
    "nixling host.json: hash-derived ifname collision detected: ${
      builtins.toJSON duplicateDerived
    }. Rename one of the colliding env/VM scopes to break the SHA-256 prefix tie.";

  envInfo = envName: m: {
    env = envName;
    bridge = m.lanBridge;
    bridgePortFlags = [
      {
        role = "net-vm-lan";
        isolated = false;
        neighSuppress = false;
        learning = true;
        unicastFlood = true;
        rule = "The net VM LAN TAP stays unisolated so workload VMs can always reach the gateway sidecar.";
      }
      {
        role = "workload-lan";
        isolated = !(lanEastWestEnabled m);
        neighSuppress = !(lanEastWestEnabled m);
        learning = true;
        unicastFlood = lanEastWestEnabled m;
        rule =
          if lanEastWestEnabled m
          then "Workload LAN TAPs are unisolated only when both lan.allowEastWest and site.allowUnsafeEastWest are true."
          else "Workload LAN TAPs stay isolated by default until both east-west opt-ins are present.";
      }
      {
        role = "uplink";
        isolated = true;
        neighSuppress = true;
        learning = false;
        unicastFlood = false;
        rule = "The host-to-net-VM uplink TAP stays point-to-point: no learning, no flooding, neighbor suppression on.";
      }
    ];
    ipv6Sysctls = map ipv6SysctlEntry (envIfNames envName m);
    usbipBusidLocks = usbipBusidLocksForEnv envName;
    lan = {
      allowEastWest = m.allowEastWest;
      effectiveEastWest = lanEastWestEnabled m;
    };
    mtu = resolvedMtu m;
    mssClamp = resolvedMssClamp m;
    netVmForwardBlocklist = m.hostBlocklist;
  };

  # ownership marker. Stable per-host id used in
  # nft rule comment markers (`comment "nixling managed: <ownership-id>"`).
  ownershipId = "nixling-${
    builtins.substring 0 8 (builtins.hashString "sha256" (
      "nixling:${toString cfg.site.allowUnsafeEastWest}:${
        builtins.concatStringsSep "," (lib.attrNames envMeta)
      }"
    ))
  }";

  data = assert lib.assertMsg (duplicateDerived == [ ]) ifNameCollisionMessage; {
    schemaVersion = "v2";
    site = {
      allowUnsafeEastWest = cfg.site.allowUnsafeEastWest;
    };
    environments = lib.mapAttrsToList envInfo envMeta;
    # 4-chain `inet nixling` layout. Plan
    # §" `inet nixling` chain layout". No raw/mangle/nat hooks.
    # All rules carry a `comment "nixling managed: <ownership-id>"`
    # marker; foreign tables/chains are never flushed.
    nftables = {
      family = "inet";
      table = "nixling";
      ownershipId = ownershipId;
      tableHashAfterApply = null;
      chains = [
        {
          name = "prerouting";
          hook = "prerouting";
          priority = -150;
          policy = "accept";
          purpose = "Filter-class prerouting chain at priority -150 (equal to mangle). Reserved for nixling-marked classification; no NAT/mangle hooks.";
        }
        {
          name = "forward";
          hook = "forward";
          priority = -5;
          policy = "drop";
          purpose = "Default-drop forward chain at priority -5 carrying per-env nixling forward policy. Foreign chains preserved untouched.";
        }
        {
          name = "output";
          hook = "output";
          priority = -5;
          policy = "accept";
          purpose = "Host-originated nixling output chain at priority -5; default accept with marked drops as needed.";
        }
        {
          name = "input";
          hook = "input";
          priority = -5;
          policy = "accept";
          purpose = "Host ingress chain at priority -5; default accept except broker-managed USBIP listener/backend drops, with runtime carve-outs inserted before those drops.";
        }
      ];
    };
    networkManager = {
      filePath = "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf";
      matchCriteria = lib.unique (lib.flatten (lib.mapAttrsToList
        (envName: m: map (ifName: "interface-name:${ifName}") (envIfNames envName m))
        envMeta));
      reloadBehavior = "Reload NetworkManager after replacing the nixling-managed unmanaged-devices file when the service is active.";
      ownership = {
        owner = "root";
        group = "root";
        mode = "0644";
        driftPolicy = "Replace only the nixling-managed generated file; do not edit foreign NetworkManager config.";
      };
    };
    hostsFile = {
      startMarker = "# nixling-managed begin";
      endMarker = "# nixling-managed end";
      rule = "Replace only the deterministic nixling-managed block between sentinels and preserve foreign /etc/hosts lines.";
    };
    kernelModules = [
      (moduleRow "kvm" "cloud-hypervisor" "required" "All nixling VMs require the base KVM module." [ ] false)
      (moduleRow "kvm_intel" "cloud-hypervisor-intel" "alternatives" "host-cpu-vendor=intel" [ ] false)
      (moduleRow "kvm_amd" "cloud-hypervisor-amd" "alternatives" "host-cpu-vendor=amd" [ ] false)
      (moduleRow "tun" "tap" "required" "All env-backed VMs require TAP devices on the host." [ ] false)
      (moduleRow "vhost_net" "virtio-net" "required" "Required when vhost-net acceleration is used for Cloud Hypervisor TAP handoff." [ ] false)
      (moduleRow "fuse" "virtiofs" "required" "Required for the per-VM virtiofs store views served by virtiofsd." [ ] false)
      (moduleRow "nf_tables" "nftables" "required" "Required for the marker-owned inet nixling table." [ ] false)
      (moduleRow "bridge" "linux-bridge" "required" "Required for every env LAN and uplink bridge." [ ] false)
      (moduleRow "br_netfilter" "bridge-netfilter" "optional" "Optional, but if present nixling fails closed unless all bridge-nf-call sysctls are zero." [
        "net.bridge.bridge-nf-call-iptables=0"
        "net.bridge.bridge-nf-call-ip6tables=0"
        "net.bridge.bridge-nf-call-arptables=0"
      ] false)
      (moduleRow "i915" "graphics-intel" "optional"
        (if anyGraphics then "Required on Intel hosts whenever any VM enables graphics." else "Used only on Intel hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "amdgpu" "graphics-amd" "optional"
        (if anyGraphics then "Required on AMD hosts whenever any VM enables graphics." else "Used only on AMD hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "nvidia" "graphics-nvidia" "optional"
        (if anyGraphics then "Required on Nvidia hosts whenever any VM enables graphics." else "Used only on Nvidia hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "nvidia_modeset" "graphics-nvidia-modeset" "optional"
        (if anyGraphics then "Required on Nvidia hosts whenever any VM enables graphics." else "Used only on Nvidia hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "nvidia_uvm" "graphics-nvidia-uvm" "optional"
        (if anyGraphics then "Required on Nvidia hosts whenever any VM enables graphics." else "Used only on Nvidia hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "nvidia_drm" "graphics-nvidia-drm" "optional"
        (if anyGraphics then "Required on Nvidia hosts whenever any VM enables graphics." else "Used only on Nvidia hosts when a VM enables graphics.")
        [ ] true)
      (moduleRow "usbip_host" "usbip" "optional"
        (if anyUsbip then "Required when host YubiKey support and at least one VM usbip.yubikey flag are both enabled." else "Used only when host YubiKey support and a VM usbip.yubikey flag are both enabled.")
        [ ] false)
      (moduleRow "vfio" "future-passthrough" "deferred" "Reserved for a later passthrough role; not required for the baseline target." [ ] false)
      (moduleRow "vhost_vsock" "future-kernel-vsock" "deferred" "Reserved for a later kernel-backed vsock path; the baseline uses Unix-socket-backed Cloud Hypervisor vsock only." [ ] false)
    ];
    fdOwnership = [
      (fdRow "/dev/kvm" "OpenKvm" "cloud-hypervisor" "SCM_RIGHTS" false "The broker opens /dev/kvm and passes the fd to the Cloud Hypervisor runner.")
      (fdRow "tap" "CreateTapFd" "cloud-hypervisor" "SCM_RIGHTS preferred; CreatePersistentTap fallback" false "The broker owns TAP creation so long-lived payloads do not retain CAP_NET_ADMIN.")
      (fdRow "/dev/vhost-net" "OpenVhostNet" "cloud-hypervisor" "SCM_RIGHTS with the TAP fd" false "vhost-net acceleration is handed off as an fd, not a visible device node.")
      (fdRow "/dev/fuse" "OpenFuse" "virtiofsd" "SCM_RIGHTS" false "virtiofsd receives /dev/fuse through the broker instead of a broad device namespace.")
      (fdRow "cgroup-dirfd" "OpenCgroupDir" "nixlingd" "SCM_RIGHTS or delegated path" false "The broker opens only the delegated nixling cgroup subtree for daemon-owned role placement.")
    ];
    runtimeProviders = runtimeProviders;
    vmRuntimes = vmRuntimeRows;
    qemuMedia =
      if qemuMediaSources == [ ] then null else {
        registryDir = "/var/lib/nixling/media-registry";
        runtimeRulesPath = "/run/udev/rules.d/99-nixling-media-ignore.rules";
        reloadBehavior = "Broker writes root-only runtime udev rules with UDISKS_IGNORE=1 and reloads udev rules after physical USB enrollment; direct image-file paths do not use enrollment.";
        sources = qemuMediaSources;
      };
    cloudHypervisorCapabilities = [
      (capabilityRow "headless" "required"
        [ "/dev/kvm" "tun" "vhost_net" "fuse" ]
        [ "virtiofsd" ]
        [ "api-socket-info" "store-virtiofs-preflight" ]
        "Headless and net-VM Cloud Hypervisor runs are the baseline target.")
      (capabilityRow "graphics" (if anyGraphics then "required" else "optional")
        [ "i915" "amdgpu" "nvidia" "nvidia_modeset" "nvidia_uvm" "nvidia_drm" ]
        [ "gpu" "video" ]
        [ "unix-socket-exists:gpu" "unix-socket-exists:video" ]
        "Graphics VMs split GPU and video helpers from the Cloud Hypervisor runner.")
      (capabilityRow "audio" (if anyAudio then "required" else "optional")
        [ "pipewire" "vhost-user-sound" ]
        [ "audio" ]
        [ "unix-socket-exists:audio" ]
        "Audio support uses a dedicated vhost-user-sound sidecar with its own profile and cgroup.")
      (capabilityRow "tpm" (if anyTpm then "required" else "optional")
        [ "swtpm" ]
        [ "swtpm" ]
        [ "unix-socket-exists:tpm-socket" ]
        "TPM support stays external to the runner and preserves the pre-start flush invariant.")
      (capabilityRow "vsock" (if anyObservability then "required" else "optional")
        [ "AF_VSOCK" "virtio-vsock" ]
        [ "vsock-relay" ]
        [ "unix-socket-exists:vsock-relay" ]
        "The baseline uses the Unix-socket-backed Cloud Hypervisor vsock path and a relay sidecar for observability flows.")
      (capabilityRow "firecracker" "deferred" [ ] [ ] [ ] "Firecracker remains a deferred non-goal pending a later ADR and parity review.")
      (capabilityRow "crosvmAsFullVmm" "deferred" [ ] [ ] [ ] "Crosvm may remain a helper, but it is not the primary VMM.")
    ];
    # hash-derived IfName mapping exposure.
    ifNameMappings = allIfNameMappings;
    # Cloud Hypervisor net-handoff probe result. The
    # broker re-runs the host-check probe at runtime and fails closed
    # with `ch-net-handoff-not-supported` if neither mode satisfies
    # declared VM resources without `CAP_NET_ADMIN`.
    ch = {
      netHandoffMode = cfg.site.ch.netHandoffMode;
    };
    # Per-host firewall coexistence policy. The default is "no managed
    # firewall detected -> coexist with whatever foreign rules exist";
    # the broker's runtime probe overrides at apply-time. Until then,
    # the static default makes the field present so downstream consumers
    # (broker, drift gate, docs) can rely on
    # `host.json.firewallCoexistencePolicy` always existing.
    firewallCoexistencePolicy = {
      manager = "none";
      policy = "coexist";
      rationale = "Default: no managed firewall manager declared on this host. The broker runtime probe overrides via ApplyNftables decisions; until then, nft rules install alongside foreign tables without flushing.";
    };
  };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-host.json" jsonText;
in
{
  options.nixling._bundle.hostJson = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal schema-v2 host.json artifact metadata.";
  };

  config = {
    nixling._bundle.hostJson = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/host.json" = privateEtc jsonFile;
  };
}
