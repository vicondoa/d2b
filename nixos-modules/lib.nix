# Shared pure helpers for the d2b framework. Imported as a
# function (`import ./lib.nix { inherit lib; }`) by network.nix and
# host.nix so they share the same MAC/IP derivation rules.
#
# Pass `pkgs` as well (`import ./lib.nix { inherit lib pkgs; }`) to
# get `d2bReadAudioState`, a Nix-store shell fragment that both
# audio.nix and cli.nix source for fail-closed audio-state reads.
{ lib, pkgs ? null }:

let
  hex2 = i:
    let s = lib.toHexString i;
    in if lib.stringLength s == 1 then "0${s}" else s;

  privateConfiguredWorkloadLimits = {
    unsafeLocal = 256;
    localVmConfigured = 256;
    total = 512;
  };

  hasConfiguredLocalVmLaunch = { realms }: row:
    let
      declared = realms.${row.realmName}.workloads.${row.workloadName};
    in
    row.kind == "local-vm"
    && row.launcherEnabled
    && (declared.launcher.items != { }
      || declared.launcher.defaultItem != null
      || declared.shell.enable);

  privateConfiguredWorkloadCounts = { rows, realms }:
    {
      unsafeLocalCount = builtins.length
        (lib.filter (row: row.kind == "unsafe-local") rows);
      localVmConfiguredCount = builtins.length
        (lib.filter (hasConfiguredLocalVmLaunch { inherit realms; }) rows);
    };

  privateConfiguredWorkloadCountAssertions =
    { unsafeLocalCount
    , localVmConfiguredCount
    , limits ? privateConfiguredWorkloadLimits
    }:
    [
      {
        assertion = unsafeLocalCount <= limits.unsafeLocal;
        message = ''
          d2b declares more than the supported maximum of ${toString limits.unsafeLocal} enabled
          unsafe-local workloads.
        '';
      }
      {
        assertion = localVmConfiguredCount <= limits.localVmConfigured;
        message = ''
          d2b declares more than the supported maximum of ${toString limits.localVmConfigured} enabled local-vm
          workloads with configured launch.
        '';
      }
      {
        assertion = unsafeLocalCount + localVmConfiguredCount <= limits.total;
        message = ''
          d2b declares more than the supported maximum of ${toString limits.total} private configured
          workloads.
        '';
      }
    ];

  hostToolOverrideKeys = [
    "d2b"
    "d2bd"
    "broker"
    "activationHelper"
    "hostActivationHelper"
    "unsafeLocalHelper"
    "resourceCompiler"
    "waylandProxy"
  ];

  formatHostToolOverrideKeys = keys:
    if keys == [ ] then "<none>" else lib.concatStringsSep ", " keys;

  validateHostToolOverrides = overrides:
    if builtins.isNull overrides then
      null
    else if !builtins.isAttrs overrides then
      throw "d2b: d2bHostToolOverrides must be null or an attribute set"
    else
      let
        keys = builtins.attrNames overrides;
        missing = lib.filter
          (key: !(builtins.elem key keys))
          hostToolOverrideKeys;
        unknown = lib.filter
          (key: !(builtins.elem key hostToolOverrideKeys))
          keys;
        nullValues = lib.filter
          (key: builtins.isNull overrides.${key})
          keys;
      in
      if keys == [ ] then
        throw "d2b: d2bHostToolOverrides must not be empty"
      else if missing != [ ] || unknown != [ ] then
        throw ''
          d2b: d2bHostToolOverrides must contain exactly the host-tool keys
          (${formatHostToolOverrideKeys hostToolOverrideKeys}); missing:
          ${formatHostToolOverrideKeys missing}; unknown:
          ${formatHostToolOverrideKeys unknown}
        ''
      else if nullValues != [ ] then
        throw ''
          d2b: d2bHostToolOverrides values must be non-null packages; null:
          ${formatHostToolOverrideKeys nullValues}
        ''
      else
        overrides;

  selectHostToolPackage =
    { overrides ? null, key, fallback }:
    if !(builtins.elem key hostToolOverrideKeys) then
      throw "d2b: unknown d2bHostToolOverrides selector key '${key}'"
    else
      let
        validatedOverrides = validateHostToolOverrides overrides;
      in
      if builtins.isNull validatedOverrides
      then fallback
      else validatedOverrides.${key};

  # d2b_read_audio_state <vm>
  # ------------------------------------------------------------
  # Fail-closed reader for /var/lib/d2b/<vm>/audio-state.json.
  # Output (one line on stdout): "mic=<on|off> speaker=<on|off>".
  # NEVER exits non-zero - callers (extraArgsScript, d2b CLI)
  # cannot handle a non-zero exit mid-flow.
  #
  # Returns "mic=off speaker=off" for EVERY error case
  #   • file missing
  #   • file present but unreadable (permissions)
  #   • file present but not valid JSON
  #   • field absent
  #   • field present but value is not the exact string "on"
  #     (e.g. boolean true, number 1, string "true", string "ON")
  #   • jq not on PATH (path is Nix-store-hardcoded below)
  #
  # The jq path is baked in at Nix eval time so the function works
  # in both audio.nix's extraArgsScript (minimal $PATH) and the
  # d2b shell application (jq also in runtimeInputs, harmless).
  d2bReadAudioState =
    if pkgs == null then null
    else
      pkgs.writeText "d2b-read-audio-state.sh" ''
        d2b_read_audio_state() {
          local _nas_vm="$1" _nas_f _nas_mic=off _nas_spk=off _nas_raw
          local _nas_canonical _nas_expected _nas_stat
          # State file lives under the root-owned state/ subdir.
          # VM state dir moved under vms/<vm>/.
          _nas_f="/var/lib/d2b/vms/$_nas_vm/state/audio-state.json"
          _nas_expected="/var/lib/d2b/vms/$_nas_vm/state/audio-state.json"
          # Canonicalize: fail closed if path doesn't resolve or is a symlink
          # pointing outside the expected location.
          _nas_canonical=$(realpath -e "$_nas_f" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_canonical" = "$_nas_expected" ] \
            || { printf 'mic=off speaker=off\n'; return 0; }
          # Verify ownership and mode: must be root:d2b 640.
          _nas_stat=$(stat -c '%U %G %a' "$_nas_canonical" 2>/dev/null) \
            || { printf 'mic=off speaker=off\n'; return 0; }
          [ "$_nas_stat" = "root d2b 640" ] \
            || { printf 'mic=off speaker=off\n'; return 0; }
          if [ -r "$_nas_canonical" ]; then
            if _nas_raw=$(${pkgs.jq}/bin/jq -re '.mic' "$_nas_canonical" 2>/dev/null) \
               && [ "$_nas_raw" = "on" ]; then
              _nas_mic=on
            fi
            if _nas_raw=$(${pkgs.jq}/bin/jq -re '.speaker' "$_nas_canonical" 2>/dev/null) \
               && [ "$_nas_raw" = "on" ]; then
              _nas_spk=on
            fi
          else
            # software-r2-1: file exists but is unreadable by the calling user
            # (e.g. an interactive operator or d2b-gpu-<vm> before ACLs are
            # applied). Fail closed so the sidecar never gets audio access on
            # a permission error.
            printf 'd2b: audio-state unreadable for %s (permission denied) - failing closed\n' "$_nas_vm" >&2
          fi
          printf 'mic=%s speaker=%s\n' "$_nas_mic" "$_nas_spk"
        }
      '';
in
rec {
  inherit hex2;
  inherit d2bReadAudioState;
  inherit hasConfiguredLocalVmLaunch privateConfiguredWorkloadCounts;
  inherit privateConfiguredWorkloadLimits privateConfiguredWorkloadCountAssertions;
  inherit selectHostToolPackage;

  cleanRustPackagesSource = packagesPath:
    lib.cleanSourceWith {
      src = packagesPath;
      filter = path: type:
        let rel = lib.removePrefix (toString packagesPath + "/") (toString path);
        in !(
          (type == "directory" && baseNameOf path == "target")
          || lib.hasInfix ".cargo/registry" rel
        );
    };

  vmRuntimeKind = vm: vm.runtime.kind or "nixos";
  isNixosVm = vm: vmRuntimeKind vm == "nixos";
  isQemuMediaVm = vm: vmRuntimeKind vm == "qemu-media";

  enabledVms = vms: lib.filterAttrs (_: vm: vm.enable) vms;
  normalNixosVms = vms: lib.filterAttrs (_: vm: vm.enable && isNixosVm vm) vms;
  qemuMediaVms = vms: lib.filterAttrs (_: vm: vm.enable && isQemuMediaVm vm) vms;

  # Keep transitional VM/env projections only for explicit legacy VM
  # references owned by enabled gateway-vm realms.
  gatewayRealms = cfg:
    lib.filterAttrs
      (_: realm:
        (realm.enable or false)
        && (realm.placement or null) == "gateway-vm")
      (cfg.realms or { });
  gatewayRealmMappings = cfg:
    let
      vms = cfg.vms or { };
      mappingFor = realm:
        let
          envNames = lib.unique (lib.filter (envName: envName != null)
            ([ (realm.env or null) ] ++ (realm.network.envs or [ ])));
          workloadVmNames = lib.unique (lib.filter
            (vmName: vmName != null)
            (map (workload: workload.legacyVmName or null)
              (lib.filter (workload: workload.enable or false)
                (lib.attrValues (realm.workloads or { })))));
          acceptedVms = lib.filterAttrs
            (name: vm:
              (vm.enable or false)
              && builtins.elem name workloadVmNames
              && builtins.elem (vm.env or null) envNames)
            vms;
          acceptedEnvNames = lib.sort lib.lessThan (lib.unique (lib.filter
            (envName: envName != null)
            (map (vm: vm.env or null) (lib.attrValues acceptedVms))));
        in {
          inherit acceptedVms acceptedEnvNames;
        };
    in
    map mappingFor (lib.attrValues (gatewayRealms cfg));
  gatewayEnvNames = cfg:
    lib.sort lib.lessThan (lib.unique (lib.concatLists
      (map (mapping: mapping.acceptedEnvNames)
        (gatewayRealmMappings cfg))));
  gatewayVmNames = cfg:
    lib.sort lib.lessThan (lib.attrNames (gatewayVms cfg));
  gatewayVms = cfg:
    lib.foldl'
      (result: mapping: result // mapping.acceptedVms)
      { }
      (gatewayRealmMappings cfg);
  gatewayEnvs = cfg:
    let envNames = gatewayEnvNames cfg;
    in lib.filterAttrs
      (name: _: builtins.elem name envNames)
      (cfg.envs or { });
  gatewayEnvMeta = cfg: envMeta:
    lib.filterAttrs
      (name: _: builtins.elem name (gatewayEnvNames cfg))
      envMeta;

  localRuntimeProvider = { id, driver }: {
    inherit id driver;
    type = "local";
  };

  nixosRuntimeProvider = localRuntimeProvider {
    id = "local-cloud-hypervisor";
    driver = "cloud-hypervisor";
  };

  qemuMediaRuntimeProvider = localRuntimeProvider {
    id = "local-qemu-media";
    driver = "qemu";
  };

  mkServiceCapability =
    { supported
    , nodeId ? null
    , runnerRole ? null
    , driver ? null
    , readiness ? null
    , contract ? null
    , transport ? null
    , unitStrategy ? null
    }:
    {
      inherit supported;
    }
    // lib.optionalAttrs (nodeId != null) { inherit nodeId; }
    // lib.optionalAttrs (runnerRole != null) { inherit runnerRole; }
    // lib.optionalAttrs (driver != null) { inherit driver; }
    // lib.optionalAttrs (readiness != null) { inherit readiness; }
    // lib.optionalAttrs (contract != null) { inherit contract; }
    // lib.optionalAttrs (transport != null) { inherit transport; }
    // lib.optionalAttrs (unitStrategy != null) { inherit unitStrategy; };

  mkRuntimeCapabilities =
    { legacy }:
    legacy;

  runtimeServiceSummary = { id, role, optional ? false }: {
    inherit id role optional;
  };

  nixosRuntimeCapabilities = mkRuntimeCapabilities {
    legacy = {
      lifecycle = true;
      display = true;
      usbHotplug = true;
      exec = true;
      configSync = true;
      ssh = true;
      storeSync = true;
      keys = true;
      inGuestObservability = true;
    };
  };

  nixosRuntimeOperationCapabilities = {
    lifecycle = {
      start = true;
      stop = true;
      restart = true;
      switch = true;
      hostPrepare = true;
    };
    media = {
      usbHotplug = true;
      removableMedia = false;
      qemuMedia = false;
    };
    display = {
      display = true;
      graphics = true;
      video = true;
      waylandProxy = true;
    };
    guest = {
      exec = true;
      shell = true;
      configSync = true;
      ssh = true;
      keys = true;
      inGuestObservability = true;
    };
    storage = {
      storeSync = true;
      virtiofs = true;
      volumes = true;
    };
  };

  qemuMediaRuntimeCapabilities = mkRuntimeCapabilities {
    legacy = {
      lifecycle = true;
      display = true;
      usbHotplug = true;
      exec = false;
      configSync = false;
      ssh = false;
      storeSync = false;
      keys = false;
      inGuestObservability = false;
    };
  };

  qemuMediaRuntimeOperationCapabilities = {
    lifecycle = {
      start = true;
      stop = true;
      restart = true;
      switch = false;
      hostPrepare = true;
    };
    media = {
      usbHotplug = true;
      removableMedia = true;
      qemuMedia = true;
    };
    display = {
      display = true;
      graphics = false;
      video = false;
      waylandProxy = false;
    };
    guest = {
      exec = false;
      shell = false;
      configSync = false;
      ssh = false;
      keys = false;
      inGuestObservability = false;
    };
    storage = {
      storeSync = false;
      virtiofs = false;
      volumes = false;
    };
  };

  nixosHypervisorService = mkServiceCapability {
    supported = true;
    nodeId = "cloud-hypervisor";
    runnerRole = "cloud-hypervisor-runner";
    driver = "cloud-hypervisor";
    readiness = "api-socket";
    contract = "spawn-runner";
    unitStrategy = "microvm-or-graphics-sidecar";
  };

  qemuMediaHypervisorService = mkServiceCapability {
    supported = true;
    nodeId = "qemu-media";
    runnerRole = "qemu-media-runner";
    driver = "qemu";
    readiness = "qmp-socket";
    contract = "spawn-runner";
    unitStrategy = "daemon-supervised-runner";
  };

  runtimeHypervisorService = kind: (runtimeProviderCatalog.${kind}
    or (throw "d2b: unsupported runtime kind '${kind}'"))._hypervisorService;

  runtimeProviderCatalog = {
    nixos = {
      kind = "nixos";
      provider = nixosRuntimeProvider;
      capabilities = nixosRuntimeCapabilities;
      operationCapabilities = nixosRuntimeOperationCapabilities;
      autostartPolicy = "host-boot-eligible";
      services = [
        (runtimeServiceSummary { id = "host-reconcile"; role = "host"; })
        (runtimeServiceSummary { id = "store-virtiofs-preflight"; role = "storage"; })
        (runtimeServiceSummary { id = "virtiofsd"; role = "storage"; })
        (runtimeServiceSummary { id = "cloud-hypervisor"; role = "hypervisor"; })
        (runtimeServiceSummary { id = "component-session"; role = "component-session"; })
        (runtimeServiceSummary { id = "swtpm"; role = "tpm"; optional = true; })
        (runtimeServiceSummary { id = "gpu"; role = "display"; optional = true; })
        (runtimeServiceSummary { id = "audio"; role = "audio"; optional = true; })
        (runtimeServiceSummary { id = "video"; role = "video"; optional = true; })
        (runtimeServiceSummary { id = "usbip"; role = "usb"; optional = true; })
      ];
      _hypervisorService = nixosHypervisorService;
    };
    qemu-media = {
      kind = "qemu-media";
      provider = qemuMediaRuntimeProvider;
      capabilities = qemuMediaRuntimeCapabilities;
      operationCapabilities = qemuMediaRuntimeOperationCapabilities;
      autostartPolicy = "manual-only";
      services = [
        (runtimeServiceSummary { id = "host-reconcile"; role = "host"; })
        (runtimeServiceSummary { id = "qemu-media"; role = "hypervisor"; })
        (runtimeServiceSummary { id = "usbip"; role = "usb"; optional = true; })
      ];
      _hypervisorService = qemuMediaHypervisorService;
    };
  };

  vmRuntimeMetadata = _name: vm:
    let
      kind = vmRuntimeKind vm;
      runtime = runtimeProviderCatalog.${kind}
        or (throw "d2b: unsupported runtime kind '${kind}'");
    in builtins.removeAttrs runtime [ "_hypervisorService" ];

  # Shared helper extracted from minijail-profiles.nix and
  # host-users.nix to eliminate the 4-line duplicate that was a
  # drift-risk for broker/ownership-matrix UID agreement. If the hash
  # algorithm or offset changes here, both consumers see the same UID,
  # preventing the ownership-matrix bug from silently returning.
  #
  # Maps a principal name (e.g. "d2b-work-aad-swtpm") to a
  # stable deterministic 24-bit UID in the range 50000..16827215.
  # `principal == "root"` short-circuits to UID 0 for the broker's
  # root-carve-out paths (ADR 0003).
  #
  # Birthday-bound collision risk: 50% at ~4096 principals,
  # 1% at ~410. Typical workstation deployments stay under 400
  # principals (≤100 VMs × 4 roles). For larger deployments,
  # extend the hash to 8 hex chars (32 bits, ~65k birthday-bound).
  # Eval-time collision detection lives in minijail-profiles.nix.
  stablePrincipalId = principal:
    if principal == "root" then 0
    else 50000 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "sha256" principal));

  # The principal uid one Device-owned worker row runs as, mirroring
  # `d2b-core`'s `mint_template_intent` (`bundle_resolver.rs`): the first
  # four bytes of the SHA-256 over
  # `<bindingOwner>:<processRef>:<executionRef>`, masked to 24 bits, offset by
  # 50000. The Device TPM Provider's state Volume and its worker must agree on
  # it, so both sides derive it from the same triple.
  deviceWorkerPrincipalId = bindingOwner: processRef: executionRef:
    50000 + (builtins.bitAnd
      (lib.fromHexString (builtins.substring 2 6
        (builtins.hashString "sha256" "${bindingOwner}:${processRef}:${executionRef}")))
      16777215);

  # The Zone names the host declares, in name order.
  zoneNames = cfg: lib.sort lib.lessThan (lib.attrNames (cfg.zones or { }));

  # The declared resource set of one Zone.
  zoneResources = cfg: zoneName: cfg.zones.${zoneName}.resources or { };

  # The declared controller execution reference of one Provider row, split into
  # its `<ResourceType>/<name>` parts, or the empty list when the row declares
  # none. The resource compiler projects a Provider's rows only against a
  # declared target, so an unresolved reference accounts for nothing on either
  # side of this boundary.
  providerTargetParts = resources: providerName:
    let
      provider = resources.${providerName} or null;
      reference =
        if provider == null then null
        else (provider.spec.config or { }).controllerExecutionRef or null;
    in
    if builtins.isString reference then lib.splitString "/" reference else [ ];

  # Whether those parts name a declared row of that same type in the Zone.
  resolvesZoneTarget = resources: parts:
    lib.length parts == 2
    && (resources.${builtins.elemAt parts 1}).type or null
      == builtins.elemAt parts 0;

  # The Provider name one declared reference carries, or the empty string when
  # the reference is not a `<ResourceType>/<name>` pair. A Device is matched
  # to its Device Provider by that name rather than by a spelled reference, so
  # this shared module carries no Provider identity of its own.
  providerNameOf = reference:
    let
      parts =
        if builtins.isString reference
        then lib.splitString "/" reference
        else [ ];
    in
    if builtins.length parts == 2 then builtins.elemAt parts 1 else "";

  # One site a Device Provider claims: the Zone, the Device, the controller
  # execution reference its worker rows bind against, and the Device settings
  # that select which worker rows the Provider's projection declares.
  #
  # A Device Provider's projection declares its worker rows only when the
  # Device is claimed by a Guest and the Provider's controller execution
  # reference resolves to a declared `Host` in the same Zone, so those are the
  # conditions here too: an account derived for a row the compiler does not
  # project would be an inert account, and an account missing for one it does
  # project would be a refused row.
  deviceWorkerSites = cfg: providerName:
    lib.concatMap
      (zoneName:
        let
          resources = zoneResources cfg zoneName;
          parts = providerTargetParts resources providerName;
        in
        if !(resolvesZoneTarget resources parts && builtins.elemAt parts 0 == "Host")
        then [ ]
        else
          map
            (device: {
              inherit zoneName device;
              executionRef = lib.concatStringsSep "/" parts;
              settings =
                (resources.${device}.spec or { }).provider.settings or { };
            })
            (lib.filter
              (name:
                (resources.${name}.type or null) == "Device"
                && providerNameOf ((resources.${name}.spec or { }).providerRef or null)
                  == providerName
                && lib.hasPrefix "Guest/" ((resources.${name}.metadata or { }).ownerRef or ""))
              (lib.attrNames resources)))
      (zoneNames cfg);

  # The longest account name the host account database carries.
  #
  # NixOS's own user and group options refuse a name of 32 bytes or more
  # (`nixos/modules/config/users-groups.nix`), and that is the POSIX bound on a
  # group name rather than a NixOS choice, so a composed name past it is not a
  # name the host holds at all.
  accountNameLimit = 31;

  # The hex digits of the digest a shortened name carries.
  accountNameHashHex = 8;

  # The account one row class runs as, bounded to what the host holds.
  #
  # A name that fits is used exactly as composed, so every ordinary row class
  # reads as the row it belongs to. One that does not keeps a readable prefix
  # of the row-class token and carries the first eight hex digits of the
  # SHA-256 over the whole composed name: two row classes that overflow
  # together stay two accounts rather than collapsing into one, and the name
  # still says which Zone and which family it belongs to. A Zone name long
  # enough that no prefix fits is a Zone this scheme cannot name either, and
  # yields no account at all - the row is then refused by the resolver, which
  # is the answer the resource compiler's own refusal gives, rather than a
  # guest that fails to evaluate.
  #
  # `d2b-core`'s `bounded_account_name` (`bundle_resolver.rs`) is this same
  # composition over the same inputs, and one table of row classes is read
  # through both (`tests/unit/nix/cases/host-worker-accounts.json`, by that
  # case and by the crate's own test), so a host that materializes these names
  # resolves the rows that carry them.
  boundedAccountName = zoneName: body:
    let
      prefix = "d2b-${zoneName}-";
      full = "${prefix}${body}";
      keep = accountNameLimit - builtins.stringLength prefix - 1 - accountNameHashHex;
    in
    if builtins.stringLength full <= accountNameLimit then
      full
    else if keep < 1 then
      null
    else
      "${prefix}${builtins.substring 0 keep body}-${builtins.substring 0 accountNameHashHex (builtins.hashString "sha256" full)}";

  # One host account row for a template-bound row class: the account name, the
  # ids it holds, and the operator-visible description - or `null` for a row
  # class whose name the host cannot carry, which every caller filters rather
  # than provisioning under a name it does not hold.
  #
  # The numeric identity is the name-derived id every other named principal in
  # this tree uses (`stablePrincipalId`), so an account keeps its ids for as
  # long as it keeps its name and a reconfigured row is never renumbered under
  # a live principal. Two schemes coexist - the Device TPM family below keeps
  # the binding-triple ids its accounts already hold - and `host-users.nix`
  # proves the two do not collide rather than assuming it.
  templateAccount = zoneName: body: description:
    let
      name = boundedAccountName zoneName body;
    in
    if name == null then
      null
    else {
      inherit name description;
      uid = stablePrincipalId name;
      gid = stablePrincipalId name;
    };

  # The host principals one zone-native Device with a TPM needs, derived from
  # the same artifacts the runtime derives them from:
  #
  # - the two worker principals the TPM Provider's state Volume grants ACL
  #   access to (`User/d2b-<zone>-<device>-swtpm` and its `-flush` sibling,
  #   `packages/d2b-provider-device-tpm/src/resources.rs`), which the
  #   state-layout effect resolves through NSS - the Volume's layout owner
  #   itself is the daemon (`User/d2bd`), and
  # - the worker-row principal uids the daemon's Device-worker tickets name
  #   (`deviceWorkerPrincipalId` over the binding's
  #   `<owner>:<rowRef>:<executionRef>` triple).
  #
  # Both worker rows of one Device share one state directory, so both
  # principals are provisioned: the long-lived swtpm worker is granted rwx on
  # it and the one-shot flush the traverse and socket writes it needs.
  deviceTpmPrincipals = cfg:
    lib.concatMap
      (site:
        let
          account = boundedAccountName site.zoneName "${site.device}-swtpm";
          flushAccount = boundedAccountName site.zoneName "${site.device}-swtpm-flush";
          ownerRef = "Provider/device-tpm";
        in
        # A Device whose name leaves the account past what the host can hold
        # contributes no principal at all, so the state Volume it would have
        # shared grants nothing and its worker rows are refused rather than
        # launched under a name the host does not hold.
        if account == null || flushAccount == null then
          [ ]
        else
          [
            {
              inherit (site) zoneName device;
              inherit account flushAccount;
              ownerUid = deviceWorkerPrincipalId ownerRef
                "Process/swtpm-${site.device}" site.executionRef;
              flushUid = deviceWorkerPrincipalId ownerRef
                "EphemeralProcess/swtpm-flush-${site.device}" site.executionRef;
            }
          ])
      (deviceWorkerSites cfg "device-tpm");

  # The Device TPM family's account rows, carrying the ids those accounts
  # already hold rather than re-deriving them: `deviceTpmPrincipals` is the
  # authority for them, because those ids predate the name-derived scheme and
  # changing them would renumber a live principal.
  deviceTpmAccounts = cfg:
    lib.concatMap
      (row: [
        {
          name = row.account;
          uid = row.ownerUid;
          gid = row.ownerUid;
          description = "d2b Device TPM state owner";
        }
        {
          name = row.flushAccount;
          uid = row.flushUid;
          gid = row.flushUid;
          description = "d2b Device TPM pre-start flush principal";
        }
      ])
      (deviceTpmPrincipals cfg);

  # The Device-owned GPU worker accounts.
  #
  # The row names are the GPU Provider's own projection's
  # (`packages/d2b-provider-device-gpu/nix/default.nix` names
  # `Process/gpu-<device>` and, for a Device that configures one,
  # `Process/video-<device>`), read here from the same Device rows and the
  # same settings that projection reads. Two accounts per Device rather than
  # one, because the GPU authority admission refuses a video principal equal
  # to the GPU principal (`PrincipalNotSeparated`): one account would make
  # that refusal unreachable.
  deviceGpuAccounts = cfg:
    lib.concatMap
      (site:
        lib.filter (row: row != null) ([
          (templateAccount site.zoneName "${site.device}-gpu"
            "d2b Device GPU worker")
        ]
        ++ lib.optional (site.settings.videoSidecar or false)
        (templateAccount site.zoneName "${site.device}-video"
          "d2b Device video decode sidecar")))
      (deviceWorkerSites cfg "device-gpu");

  # The account one Provider's controller rows run as.
  #
  # A controller row's name is a hash over its Zone, Provider, component and
  # execution target, so it is not a name an account can be derived from and
  # not a boundary worth separating on: one Provider is one signed artifact
  # published under one key, and its components share that trust. The Provider
  # is therefore the account's granularity, and the Zone stays in the name so
  # one Zone's controllers never share an identity with another's. The
  # compiler projects a controller row only for a Provider that declares a
  # controller execution reference resolving to a declared `Host` or `Guest`
  # in its own Zone, so an account is derived on exactly that condition.
  providerControllerAccounts = cfg:
    lib.concatMap
      (zoneName:
        let
          resources = zoneResources cfg zoneName;
        in
        lib.filter (row: row != null) (map
          (providerName:
            templateAccount zoneName "controller-${providerName}"
              "d2b Provider controller")
          (lib.filter
            (name:
              (resources.${name}.type or null) == "Provider"
              && resolvesZoneTarget resources
                (providerTargetParts resources name))
            (lib.attrNames resources))))
      (zoneNames cfg);

  # The account the binding-owned virtiofsd serving worker runs as.
  #
  # Its launch ticket has the two path trees it names opened to its principal,
  # so it does not share the controller account of the Provider that declares
  # it. A Zone declares that Provider only when it has the row: the resource
  # compiler emits no serving-worker template without it.
  zoneTemplateAccounts = cfg:
    lib.concatMap
      (zoneName:
        let
          resources = zoneResources cfg zoneName;
          declares = providerName: (resources.${providerName}.type or null) == "Provider";
        in
        lib.filter (row: row != null) (lib.optional (declares "volume-virtiofs")
          (templateAccount zoneName "virtiofsd" "d2b Zone serving worker")))
      (zoneNames cfg);

  # Every host account a template-bound row class runs as, derived per Zone
  # from the same trusted Zone rows the resource compiler binds.
  #
  # `d2b-core`'s `template_account` (`packages/d2b-core/src/bundle_resolver.rs`)
  # composes each of these names from the binding's own owner reference,
  # declared row name, and template. Every input here is one of those, read
  # out of the same `cfg.zones.<zone>.resources` set, so the two sides are two
  # evaluators of one rule over one input rather than two naming schemes kept
  # in agreement by hand:
  #
  # | row class | account |
  # |---|---|
  # | Device TPM worker | `d2b-<zone>-<device>-swtpm` |
  # | its one-shot pre-start flush | `d2b-<zone>-<device>-swtpm-flush` |
  # | Device GPU worker | `d2b-<zone>-<device>-gpu` |
  # | its video decode sidecar | `d2b-<zone>-<device>-video` |
  # | a Provider's controller rows | `d2b-<zone>-controller-<provider>` |
  # | the binding-owned serving worker | `d2b-<zone>-virtiofsd` |

  # Each of those names is the `d2b-<zone>-<row-class token>` `boundedAccountName`
  # composes, so a row class whose composed name is longer than the 31 bytes
  # the host account database carries keeps a readable prefix of that token
  # and carries eight hex digits of the SHA-256 over the whole name instead.
  # The two sides shorten the same way over the same input, so the row still
  # resolves; a Zone name long enough that nothing fits is refused by the
  # resolver rather than provisioned under a name the host does not hold.
  #
  # A row class neither side names has no account: the credential agent a
  # `Credential` controller adopts is one the shared crate may not name under
  # the Provider family-knowledge rule, so its rows stay refused here rather
  # than resolving to an account whose name only one side holds.
  templateWorkerAccounts = cfg:
    deviceTpmAccounts cfg
    ++ deviceGpuAccounts cfg
    ++ providerControllerAccounts cfg
    ++ zoneTemplateAccounts cfg;

  # Stable virtio-blk serial for a d2b.vms.<vm>.runner.volumes entry.
  # Cloud Hypervisor emits this into the block device, while
  # vm-guest-base.nix mounts by the corresponding
  # /dev/disk/by-id/virtio-<serial> path.
  volumeSerial = volume:
    if (volume.serial or null) != null then volume.serial else (
      let
        base = baseNameOf (toString volume.image);
        withoutImg = lib.removeSuffix ".img" base;
        sanitized = lib.replaceStrings [ "." "_" "/" " " "," "=" ] [ "-" "-" "-" "-" "-" "-" ] withoutImg;
      in
      if sanitized == "" then "disk" else sanitized
    );

  volumeHostPath = stateDir: vmName: volume:
    let image = toString volume.image; in
    if lib.hasPrefix "/" image then image else "${toString stateDir}/${vmName}/${image}";

  volumeDiskInitEligible = volume:
    !(lib.hasPrefix "/" (toString volume.image))
    && (toString (volume.imageType or "raw")) == "raw"
    && (toString (volume.fsType or "ext4")) == "ext4";

  volumeSizeBytes = volume: (volume.size or 1024) * 1024 * 1024;

  volumeFileSystem = volume: {
    device = "/dev/disk/by-id/virtio-${volumeSerial volume}";
    fsType = volume.fsType or "ext4";
    options = [ "x-systemd.after=systemd-modules-load.service" ]
      ++ lib.optional (volume.readOnly or false) "ro";
    neededForBoot = true;
  };

  componentSessionVsockPort = 14318;
  observabilityOtlpVsockPort = 14317;
  # AF_VSOCK port used by the d2b security-key CTAPHID relay frontend.
  # The guest sk-frontend connects on this port to the host broker.
  securityKeyVsockPort = 14320;
  observabilityStackVsockCid = 1000;

  # Deterministic per-VM Cloud Hypervisor vsock CID. Env-backed VMs
  # reserve slot 1 for the env net VM and use d2b.vms.<vm>.index
  # for workloads (10..250). The stride intentionally exceeds the
  # maximum workload index so adjacent envs cannot collide.
  componentSessionVsockCid = { name, envIndex ? null, index ? null, isNetVm ? false, isObservabilityVm ? false }:
    if isObservabilityVm then observabilityStackVsockCid
    else if envIndex != null then
      let slot = if isNetVm then 1 else index; in
      100 + (envIndex * 1000) + slot
    else
      4096 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "md5" name));

  componentSessionVsockHostSocket = stateRoot: "${stateRoot}/vsock.sock";

  volumeSerialIssues = volumes:
    let
      serials = map volumeSerial volumes;
    in {
      duplicates = lib.unique (lib.filter
        (serial: lib.count (candidate: candidate == serial) serials > 1)
        serials);
      reserved = lib.filter (serial: serial == "rootfs") serials;
      tooLong = lib.filter (serial: lib.stringLength serial > 20) serials;
      unsafe = lib.filter
        (serial: builtins.match "^[A-Za-z0-9][A-Za-z0-9-]{0,19}$" serial == null)
        serials;
    };

  # subnetIp "10.20.0.0/24" 5  =>  "10.20.0.5"
  # subnetIp "192.0.2.252/30" 1 => "192.0.2.1"  (host-octet only,
  # caller knows the prefix length)
  subnetIp = subnet: octet:
    let
      base = builtins.head (lib.splitString "/" subnet);
      parts = lib.splitString "." base;
      first3 = lib.take 3 parts;
    in
    lib.concatStringsSep "." (first3 ++ [ (toString octet) ]);

  subnetPrefix = subnet: builtins.head (lib.splitString "/" subnet);
  subnetMask = subnet: lib.last (lib.splitString "/" subnet);

  # Parse "10.0.0.0/24" → { netInt = 167772160; prefix = 24; }
  # Used by cidrOverlaps below. Pure Nix - no shell, no `ip`
  # spawning at eval time. Assumes a well-formed IPv4 CIDR; the
  # callers in network.nix already gate per-env shape (/24 lan, /30
  # uplink,.0 network address). cfg.hostLanCidrs is consumer-set;
  # the helper still parses it correctly for any IPv4 CIDR.
  parseCidr = cidr:
    let
      parts = lib.splitString "/" cidr;
      octets = lib.splitString "." (builtins.head parts);
      prefix =
        if lib.length parts == 2
        then lib.toInt (lib.last parts)
        else 32;
      netInt =
        lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
    in
    { inherit netInt prefix; };

  # cidrOverlaps "10.0.0.0/24" "10.0.0.128/26" = true
  # cidrOverlaps "10.0.0.0/24" "10.0.1.0/24"   = false
  # cidrOverlaps "10.0.0.0/24" "10.0.0.0/16"   = true  (containment)
  # cidrOverlaps "10.0.0.0/24" "192.168.1.0/24" = false
  #
  # Two CIDRs overlap iff their broader prefix matches on both
  # network addresses. We compare top-N bits where N = min(prefixA,
  # prefixB) by shifting both netInts right by (32 - N) via integer
  # division. No explicit mask construction needed.
  cidrOverlaps = a: b:
    let
      A =
        let
          parts = lib.splitString "/" a;
          octets = lib.splitString "." (builtins.head parts);
          prefix =
            if lib.length parts == 2
            then lib.toInt (lib.last parts)
            else 32;
          netInt =
            lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
        in
        { inherit netInt prefix; };
      B =
        let
          parts = lib.splitString "/" b;
          octets = lib.splitString "." (builtins.head parts);
          prefix =
            if lib.length parts == 2
            then lib.toInt (lib.last parts)
            else 32;
          netInt =
            lib.foldl' (acc: o: acc * 256 + lib.toInt o) 0 octets;
        in
        { inherit netInt prefix; };
      minPrefix = if A.prefix < B.prefix then A.prefix else B.prefix;
      shift = 32 - minPrefix;
      pow2 = n:
        lib.foldl' (acc: _: acc * 2) 1 (lib.genList (i: i) n);
      divisor = pow2 shift;
      aTop = A.netInt / divisor;
      bTop = B.netInt / divisor;
    in
    aTop == bTop;

  # Deterministic MAC: 02:<hash(env+ifaceSuffix)[0..8]>:<index hex>.
  # `02` = locally-administered, unicast. Last byte = index. The
  # ifaceSuffix lets a single VM with two NICs (router VMs) get two
  # distinct MACs without index collisions: pass "up" for the
  # uplink-side NIC and "lan" for the LAN-side NIC.
  mkMac = env: ifaceSuffix: index:
    let
      h = builtins.substring 0 8 (builtins.hashString "sha256" "${env}-${ifaceSuffix}");
      pair = n: builtins.substring n 2 h;
    in
    lib.toUpper "02:${pair 0}:${pair 2}:${pair 4}:${pair 6}:${hex2 index}";

  # vmRunner - single access point for per-VM runner config that
  # guest-closures.nix / store.nix consume. Reads from
  # `config.d2b._computed.<name>.config.d2b.vms.<name>.runner.*` -
  # the d2b-owned per-VM evaluator output (see
  # `nixos-modules/vm-evaluator.nix`). The
  # `d2b._computed.<name>` storage location is a SIBLING
  # to `d2b.vms.<name>` to avoid module-system infinite
  # recursion (the composeVm pass cannot map over cfg.vms
  # and write back to d2b.vms.<name>.computed without
  # cycling). No upstream dependency.
  vmRunner = config: name:
    config.d2b._computed.${name}.config.d2b.vms.${name}.runner or { };

  # Sibling helper for the per-VM toplevel build.
  vmToplevel = config: name:
    config.d2b._computed.${name}.config.system.build.toplevel;

  # Sibling helper for the per-VM declared runner derivation.
  # In v1.1+ this is always null (the broker generates runner
  # argv from the trusted bundle; provider-specific generators remain
  # in their owning Provider crates. The helper returns null for
  # backward compat with consumers that touch the path.
  vmDeclaredRunner = _config: _name: null;

  v3GuestSystemFor = guestSystems: zone: name:
    let
      byZone =
        if builtins.isAttrs guestSystems
          && builtins.hasAttr zone guestSystems
          && builtins.isAttrs guestSystems.${zone}
        then guestSystems.${zone}
        else { };
    in
    if builtins.hasAttr name byZone
    then byZone.${name}
    else null;

  v3GuestConfigFor = guestSystem:
    if guestSystem == null then null
    else if builtins.isAttrs guestSystem && builtins.hasAttr "config" guestSystem
    then guestSystem.config
    else if builtins.isAttrs guestSystem
    then guestSystem
    else null;

  v3GuestEvaluatorReady = guestSystem:
    let
      guestConfig = v3GuestConfigFor guestSystem;
      guestSystemConfig =
        if builtins.isAttrs guestConfig
          && builtins.hasAttr "system" guestConfig
        then guestConfig.system
        else { };
      guestBuild =
        if builtins.isAttrs guestSystemConfig
          && builtins.hasAttr "build" guestSystemConfig
        then guestSystemConfig.build
        else { };
    in
    builtins.isAttrs guestConfig
    && builtins.isAttrs guestSystemConfig
    && builtins.isAttrs guestBuild
    && builtins.hasAttr "toplevel" guestBuild;

  v3GuestRows =
    { zones
    , guestSystems ? { }
    , artifacts ? { }
    }:
    lib.concatMap
      (zoneName:
        let zone = zones.${zoneName};
        in lib.mapAttrsToList
          (resourceName: resource:
            let
              spec = resource.spec or { };
              artifactId = spec.systemArtifactId or null;
            in {
              inherit zoneName resourceName resource spec;
              system = v3GuestSystemFor guestSystems zoneName resourceName;
              artifact =
                if artifactId != null
                  && builtins.isAttrs artifacts
                  && builtins.hasAttr artifactId artifacts
                then artifacts.${artifactId}
                else null;
            })
          (lib.filterAttrs (_: resource: resource.type == "Guest") zone.resources))
      (lib.sort lib.lessThan (lib.attrNames zones));

  # guestConfigForbiddenNamespaces - namespace-containment policy check
  # for the per-VM guest-editable `guestConfigFile`.
  #
  # Returns the host-owned option path(s) (under `d2b.*`) that the
  # guest file - OR ANY MODULE IT IMPORTS / GENERATES - defined. An
  # empty list means the guest file touched only guest-OS options.
  #
  # Mechanism: evaluate the guest file (and its full import closure) with
  # `lib.evalModules` over the REAL nixpkgs NixOS module set, so a guest
  # module that READS a standard option (e.g.
  # `config.networking.hostName` in a `mkIf` guard) resolves instead of
  # crashing the host eval. `d2b` is redeclared as a detector option
  # that nothing else defines, and a namespace is reported iff
  # `options.d2b.isDefined` - i.e. the guest contributed a real
  # definition. Detection is by definition-EXISTENCE, so a guest's
  # `imports`, a `builtins.toFile`-generated module, and `_file`
  # spoofing are all caught (none can hide a definition from the option
  # system). The retired upstream option namespace needs no detector
  # root: it no longer exists, so a guest file setting it fails the
  # sandbox eval as an unknown option and is reported fail-closed.
  #
  # SCOPE / NON-GOAL: this is a best-effort namespace-containment policy
  # lint, NOT a sound eval-time security sandbox. Two known limits, both
  # inherent to evaluating guest-authored Nix as a module and both with
  # the SAME backstop (the operator-review-and-approve trust gate - the
  # host only ever evaluates a guest file the operator reviewed via
  # `config diff` and approved) and the SAME deferred sound fix (a
  # restricted/pure evaluator whose normalized output is the only thing
  # the host consumes - see docs/adr/0024 "Future work"):
  #   1. `lib.evalModules` cannot stop an approved guest file from
  #      reading host paths at eval time (e.g. `builtins.readFile`).
  #   2. This lint evaluates the guest file over the base NixOS module
  #      set, NOT the full per-VM module stack (components, framework).
  #      So the `config.*` context can differ from the real eval, which
  #      has two consequences:
  #      (a) a forbidden `d2b.*` definition gated on
  #          `lib.mkIf <cond>` where `<cond>` depends on a value the real
  #          eval sets but this context does not can evaluate false here
  #          and true in the real eval, escaping the lint (false NEGATIVE);
  #      (b) a contained guest file that READS a framework-declared option
  #          (e.g. `config.d2b.sshUser`, declared by the framework but
  #          here only an undefined `anything` detector root) fails the
  #          sandbox eval and is reported fail-closed (false POSITIVE).
  #      Sound attribution of a guest's contribution in the FULL context
  #      is not reliably expressible - definition counts conflate
  #      defaults, value comparison forces (and is perturbed by) the whole
  #      runner closure, and source-file attribution is `_file`-spoofable
  #      - hence the deferred restricted-evaluator.
  # The lint reliably catches the common case: UNCONDITIONAL host-owned
  # sets, and conditional ones whose guard resolves the same here as in
  # the real eval - from any source (`imports`, `builtins.toFile`,
  # `_file` spoofing), since detection is by definition-existence. Guest
  # files that read framework-declared `d2b.*` options are not supported
  # (they read host-owned state the guest layer should not depend on).
  #
  # `pkgs` + `specialArgs` mirror what the real per-VM evaluator passes
  # so a guest config valid in the real eval applies here too. Any eval
  # failure is treated fail-closed (reported as a violation).
  guestConfigForbiddenNamespaces = { pkgs, specialArgs ? { } }: guestFile:
    let
      modulesPath = toString (pkgs.path + "/nixos/modules");
      baseModules = import (modulesPath + "/module-list.nix");
      ev = lib.evalModules {
        specialArgs = {
          inherit lib pkgs modulesPath baseModules;
          utils = import (pkgs.path + "/nixos/lib/utils.nix") {
            inherit lib pkgs;
            config = ev.config;
          };
        } // specialArgs;
        modules = baseModules ++ [
          {
            nixpkgs.pkgs = pkgs;
            nixpkgs.hostPlatform = pkgs.stdenv.hostPlatform.system;
          }
          {
            options.d2b = lib.mkOption { type = lib.types.anything; };
          }
          guestFile
        ];
      };
      namesIn = ns:
        lib.optionals ev.options.${ns}.isDefined
          (lib.concatMap
            (def: map (k: "${ns}.${k}") (lib.attrNames def))
            ev.options.${ns}.definitions);
      probe = builtins.tryEval (namesIn "d2b");
    in
    if probe.success then probe.value
    else [ "<guestConfigFile failed to evaluate in the containment check>" ];
}
