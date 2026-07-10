# Eval-time validation of the d2b option schema.
#
# All assertions here fire BEFORE any build kicks off, so a typo in
# `d2b.vms.<name>` or an unsupported env name turns into a clear
# eval error instead of a confusing systemd-escape failure or a
# 16-char-truncated `ip link` name at runtime.
#
# The matching env-level assertions (env exists, env+index uniqueness,
# staticIp / env mutual exclusion, env name ≤ 8 chars) live in
# network.nix where the env iteration happens. This file owns the
# per-VM-name and per-env-name format / reserved-prefix checks that
# don't depend on network.nix's iteration of cfg.envs.
{ config, lib, options, pkgs, ... }:

let
  cfg = config.d2b;
  obsCfg = cfg.observability;
  obsVsockCid = 1000;
  u32Max = 4294967295;
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) cidrOverlaps parseCidr subnetIp volumeSerialIssues;

  pow2 = n:
    lib.foldl' (acc: _: acc * 2) 1 (lib.genList (i: i) n);

  cidrContains = cidr: ip:
    let
      parsed = parseCidr cidr;
      divisor = pow2 (32 - parsed.prefix);
      ipInt = (parseCidr ip).netInt;
    in
    parsed.netInt / divisor == ipInt / divisor;

  # Allowed SSH public-key types. We match by prefix on the key line
  # ("ssh-ed25519 AAAA..."). Adding/removing types here is a deliberate
  # choice — be conservative: legacy RSA<2048, DSA, etc. are NOT in
  # this list.
  sshKeyPrefixes = [
    "ssh-ed25519 "
    "ssh-rsa "
    "ecdsa-sha2-nistp256 "
    "ecdsa-sha2-nistp384 "
    "ecdsa-sha2-nistp521 "
    "sk-ssh-ed25519@openssh.com "
    "sk-ecdsa-sha2-nistp256@openssh.com "
  ];

  hasAllowedSshPrefix = s:
    lib.any (p: lib.hasPrefix p s) sshKeyPrefixes;

  # Eval-time validation of an authorized-keys entry. The entry is
  # either a path (a.pub file on disk, e.g. /etc/nixos/keys/foo.pub)
  # or a string (the literal pubkey content). Either way the first
  # non-empty / non-comment line must start with an allowed prefix
  # AND must not look like a PRIVATE key (the universal disaster
  # signal: "-----BEGIN ... PRIVATE KEY-----").
  authorizedKeyEntryOk = entry:
    let
      raw =
        if builtins.isPath entry || lib.isStorePath entry
        then builtins.readFile entry
        else if builtins.isString entry
        then entry
        else throw "d2b: authorized-key entry must be a path or string";
      lines = lib.splitString "\n" raw;
      # Strip pure-comment and pure-whitespace lines.
      firstReal = lib.findFirst
        (l: let s = lib.removePrefix " " l;
            in s != "" && !(lib.hasPrefix "#" s))
        ""
        lines;
      noPrivateMarker = !(lib.hasInfix "PRIVATE KEY" raw);
    in
    noPrivateMarker && hasAllowedSshPrefix firstReal;

  # Pretty origin for the error message — for a path, name it; for a
  # string, truncate.
  authorizedKeyEntryName = entry:
    if builtins.isPath entry || lib.isStorePath entry
    then toString entry
    else "<literal key string '${lib.substring 0 40 (toString entry)}…'>";

  validateAuthorizedKeys = label: list:
    lib.flatten (lib.imap0
      (i: entry: lib.optional (!authorizedKeyEntryOk entry) {
        assertion = false;
        message = ''
          ${label} entry #${toString i} (${authorizedKeyEntryName entry})
          does not look like a valid SSH public key.

          Allowed types: ed25519, RSA, ECDSA (nistp256/384/521),
          security-key variants.

          Common causes:
          - You handed it a PRIVATE key file (a "-----BEGIN ... PRIVATE
            KEY-----" header was found in the content). Use the
            matching .pub file instead.
          - The file is empty or only contains comments.
          - The key uses an unsupported type (legacy RSA<2048, DSA, ...).
        '';
      })
      list);

  # Auto-declared system VMs (added to cfg.vms by network.nix and, when
  # enabled, by observability-vm.nix) must NOT trip the `sys-`
  # reserved-prefix rule. Derive the allowed set from the auto-net VMs
  # plus the reserved observability stack VM name.
  autoGatewayVmNames =
    lib.mapAttrsToList
      (_: gw: gw.vmName)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);
  enabledGateways = lib.mapAttrsToList
    (name: gw: { inherit name gw; })
    (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);
  enabledGatewayNames = map (gateway: gateway.name) enabledGateways;
  enabledGatewayRealms = map (gateway: gateway.gw.realm) enabledGateways;
  enabledGatewayEnvs = map (gateway: gateway.gw.env) enabledGateways;
  enabledGatewayVmNames = map (gateway: gateway.gw.vmName) enabledGateways;
  duplicateValues = values:
    let
      countValue = value: lib.length (lib.filter (candidate: candidate == value) values);
    in
    lib.unique (lib.filter (value: countValue value > 1) values);
  duplicateGatewayRealms = duplicateValues enabledGatewayRealms;
  duplicateGatewayEnvs = duplicateValues enabledGatewayEnvs;
  duplicateGatewayVmNames = duplicateValues enabledGatewayVmNames;

  realmIndex = cfg._index.realms;
  realmRows = realmIndex.list;
  enabledRealmRows = realmIndex.enabledList;
  duplicateRealmIds = duplicateValues (map (realm: realm.id) realmRows);
  duplicateRealmPaths = duplicateValues (map (realm: realm.path) realmRows);
  duplicateEnabledRealmPathValues = field:
    duplicateValues (map (realm: realm.paths.${field}) enabledRealmRows);
  realmPathCollisionFields = [
    "stateDir"
    "auditDir"
    "runDir"
    "publicSocket"
    "brokerSocket"
  ];
  realmUnixSocketFields = [
    "publicSocket"
    "brokerSocket"
  ];
  providerBackedRealmPlacements = [
    "provider-controller"
    "provider-agent"
    "provider-specific"
  ];
  localRealmPlacements = [
    "host-local"
    "gateway-vm"
    "cloud-full-host"
  ];

  # Realm workload assertions.
  #
  # Cross-realm vsock CID collision: two workloads in DIFFERENT realms
  # referencing DIFFERENT local NixOS VMs whose derived vsock CIDs collide.
  # (Same-VM references across realms share a CID by design and are not
  # flagged here; the global vmVsockCidCollisions check covers per-VM uniqueness.)
  realmWorkloadRows = realmIndex.workloads.enabled;
  nixosWorkloadRows =
    lib.filter
      (row:
        row.legacyVmName != null
        && row.runtimeKind == "nixos"
        && builtins.hasAttr row.legacyVmName (d2bLib.normalNixosVms cfg.vms))
      realmWorkloadRows;
  nixosWorkloadCidPairs =
    map
      (row: {
        realmName = row.realmName;
        workloadName = row.workloadName;
        legacyVmName = row.legacyVmName;
        cid = config.d2b.manifest.${row.legacyVmName}.observability.vsockCid;
      })
      nixosWorkloadRows;
  nixosWorkloadCidGroups = lib.groupBy (p: "${toString p.cid}") nixosWorkloadCidPairs;
  crossRealmWorkloadCidCollisions =
    lib.flatten (lib.mapAttrsToList
      (_: pairs:
        let
          # Only flag when the collision involves more than one distinct realm
          realms = lib.unique (map (p: p.realmName) pairs);
          # And more than one distinct VM (same-VM cross-realm shares a CID intentionally)
          vms = lib.unique (map (p: p.legacyVmName) pairs);
        in
        lib.optional (lib.length realms > 1 && lib.length vms > 1) {
          cid = (builtins.head pairs).cid;
          pairs = map (p: { inherit (p) realmName workloadName legacyVmName; }) pairs;
        })
      nixosWorkloadCidGroups);

  # Cross-realm external network attachment conflict: two or more realms
  # whose network envs share the same attachment interface. Advisory only
  # in metadata-only runtime state; the index exposes the same data via
  # cfg._index.realms.externalNetworkConflicts.
  crossRealmExtNetConflicts = cfg._index.realms.externalNetworkConflicts;

  # Realm-native network port-forward assertions.

  # Flat list of { realmName, pf } pairs for all declared portForwards.
  realmPortForwardPairs = lib.flatten (lib.mapAttrsToList
    (realmName: realm:
      map (pf: { inherit realmName pf; }) realm.network.externalNetwork.portForwards)
    (lib.filterAttrs (_: realm: realm.enable) cfg.realms));

  # (1) Each portForward must specify exactly one of workload or targetIp.
  portForwardBothOrNeitherRows = lib.filter
    (entry:
      let pf = entry.pf;
      in (pf.workload != null && pf.targetIp != null) ||
         (pf.workload == null && pf.targetIp == null))
    realmPortForwardPairs;

  # (2) attachment.enable is required whenever egress, portForwards, or mdns
  # are configured (the net VM needs an external NIC to route those flows).
  realmsMissingAttachment = lib.filter
    (realmName:
      let realm = cfg.realms.${realmName};
      in realm.enable
        && (realm.network.externalNetwork.egress.enable
            || realm.network.externalNetwork.portForwards != []
            || realm.network.externalNetwork.mdns.enable)
        && !realm.network.externalNetwork.attachment.enable)
    (builtins.attrNames cfg.realms);

  # (3) A portForward that names a workload must name one declared in the same realm.
  portForwardMissingWorkloadRows = lib.filter
    (entry:
      let pf = entry.pf;
          realm = cfg.realms.${entry.realmName};
      in pf.workload != null
         && !(builtins.hasAttr pf.workload realm.workloads))
    realmPortForwardPairs;

  # (4) Unsafe-local workloads have no guest address behind the realm net VM.
  portForwardUnsafeLocalRows = lib.filter
    (entry:
      let
        pf = entry.pf;
        realm = cfg.realms.${entry.realmName};
        workload =
          if pf.workload == null then null
          else realm.workloads.${pf.workload} or null;
      in workload != null && workload.kind == "unsafe-local")
    realmPortForwardPairs;

  realmPortForwardAssertions =
    map
      (entry: {
        assertion = false;
        message = ''
          d2b.realms.${entry.realmName}.network.externalNetwork.portForwards:
          port-forward on ${entry.pf.protocol}/${toString entry.pf.listenPort}
          must specify exactly one of `workload` or `targetIp`, not both and
          not neither. Set `workload = "<name>"` to route to a declared
          workload in this realm, or `targetIp = "<ip>"` for an explicit IP.
        '';
      })
      portForwardBothOrNeitherRows
    ++ map
      (realmName: {
        assertion = false;
        message = ''
          d2b.realms.${realmName}.network.externalNetwork:
          attachment.enable must be true when egress.enable, portForwards, or
          mdns.enable are configured. The net VM requires an external NIC
          (attachment) to route egress, forward ports, or reflect mDNS.
          Set d2b.realms.${realmName}.network.externalNetwork.attachment.enable = true.
        '';
      })
      realmsMissingAttachment
    ++ map
      (entry: {
        assertion = false;
        message = ''
          d2b.realms.${entry.realmName}.network.externalNetwork.portForwards:
          workload "${entry.pf.workload}" on ${entry.pf.protocol}/${toString entry.pf.listenPort}
          is not declared in d2b.realms.${entry.realmName}.workloads.
          Declare the workload first, or use `targetIp` instead of `workload`
          for an explicit IP destination.
        '';
      })
      portForwardMissingWorkloadRows
    ++ map
      (entry: {
        assertion = false;
        message = ''
          d2b.realms.${entry.realmName}.network.externalNetwork.portForwards:
          workload "${entry.pf.workload}" is unsafe-local and has no guest
          network address behind the realm net VM. Net-VM port forwards can
          target VM-backed workloads or an explicit `targetIp`, not host-user
          processes.
        '';
      })
      portForwardUnsafeLocalRows;


  missingRealmParents = lib.filter
    (realm:
      realm.enabled
      && realm.parentPath != null
      && !(builtins.hasAttr realm.parentPath realmIndex.enabledByPath))
    enabledRealmRows;

  realmParentCycleFor = realm:
    let
      maxDepth = (lib.length enabledRealmRows) + 1;
      step = state: _:
        if state.done then
          state
        else
          let
            currentRow = realmIndex.enabledByPath.${state.current};
            parent = currentRow.parentPath;
          in
          if parent == null || !(builtins.hasAttr parent realmIndex.enabledByPath) then
            state // { done = true; }
          else if builtins.elem parent state.seen then
            {
              done = true;
              current = parent;
              seen = state.seen ++ [ parent ];
              cycle = state.seen ++ [ parent ];
            }
          else
            {
              done = false;
              current = parent;
              seen = state.seen ++ [ parent ];
              cycle = null;
            };
      final = lib.foldl' step
        {
          done = false;
          current = realm.path;
          seen = [ realm.path ];
          cycle = null;
        }
        (lib.genList (i: i) maxDepth);
    in
    final.cycle;
  realmParentCycles = lib.unique
    (lib.filter (cycle: cycle != null)
      (map realmParentCycleFor enabledRealmRows));
  realmSocketPathTooLongRows = field:
    lib.filter
      (realm: builtins.stringLength realm.paths.${field} > 107)
      enabledRealmRows;
  realmMissingPlacementProviderRows = lib.filter
    (realm:
      builtins.elem realm.placement providerBackedRealmPlacements
      && realm.placementProvider == null)
    enabledRealmRows;
  realmUnexpectedPlacementProviderRows = lib.filter
    (realm:
      builtins.elem realm.placement localRealmPlacements
      && realm.placementProvider != null)
    enabledRealmRows;

  realmAssertions = [
    {
      assertion = duplicateRealmIds == [ ];
      message = ''
        d2b.realms must use unique stable realm ids. Duplicate id(s): ${
          lib.concatStringsSep ", " duplicateRealmIds
        }.
      '';
    }
    {
      assertion = duplicateRealmPaths == [ ];
      message = ''
        d2b.realms must use unique canonical realm paths. Duplicate path(s): ${
          lib.concatStringsSep ", " duplicateRealmPaths
        }.
      '';
    }
    {
      assertion = missingRealmParents == [ ];
      message = ''
        enabled child realms must name an enabled parent realm by canonical
        path. Missing or disabled parent reference(s): ${
          lib.concatStringsSep ", " (map
            (realm: "${realm.path} -> ${realm.parentPath}")
            missingRealmParents)
        }.
      '';
    }
    {
      assertion = realmParentCycles == [ ];
      message = ''
        enabled d2b.realms parent links must form an acyclic tree. Cycle(s): ${
          lib.concatStringsSep "; " (map
            (cycle: lib.concatStringsSep " -> " cycle)
            realmParentCycles)
        }.
      '';
    }
    {
      assertion = realmMissingPlacementProviderRows == [ ];
      message = ''
        provider-backed d2b.realms placements require
        `placementProvider` so the realm can map to
        RealmControllerPlacement.provider. Set
        d2b.realms.<realm>.placementProvider for provider-controller,
        provider-agent, and provider-specific realm(s): ${
          lib.concatStringsSep ", " (map (realm: "${realm.path} (${realm.placement})") realmMissingPlacementProviderRows)
        }.
      '';
    }
    {
      assertion = realmUnexpectedPlacementProviderRows == [ ];
      message = ''
        d2b.realms.<realm>.placementProvider is valid only for provider-backed
        placements (provider-controller, provider-agent, provider-specific).
        Leave it null for host-local, gateway-vm, and cloud-full-host realm(s): ${
          lib.concatStringsSep ", " (map (realm: "${realm.path} (${realm.placement})") realmUnexpectedPlacementProviderRows)
        }.
      '';
    }
  ] ++ map
    (field:
      let overlong = realmSocketPathTooLongRows field;
      in {
        assertion = overlong == [ ];
        message = ''
          d2b.realms.<realm>.paths.${field} must fit Linux AF_UNIX pathname
          sockets: at most 107 bytes including path characters, leaving one
          byte for the terminating NUL in sockaddr_un.sun_path. Overlong
          realm socket path(s): ${
            lib.concatStringsSep ", " (map (realm: "${realm.path} (${toString (builtins.stringLength realm.paths.${field})} bytes)") overlong)
          }.
        '';
      })
    realmUnixSocketFields
  ++ map
    (field:
      let duplicates = duplicateEnabledRealmPathValues field;
      in {
        assertion = duplicates == [ ];
        message = ''
          enabled d2b.realms must not share ${field} paths. Duplicate path(s): ${
            lib.concatStringsSep ", " duplicates
          }.
        '';
      })
    realmPathCollisionFields
  # Cross-realm workload vsock CID collision assertion.
  # Fires when two workloads in different realms reference different NixOS VMs
  # whose derived vsock CIDs collide. Same-realm or same-VM cross-realm
  # references are not flagged here; the global vmVsockCidCollisions gate
  # covers all per-VM CID uniqueness.
  ++ map
    (collision: {
      assertion = false;
      message = ''
        Cross-realm vsock CID collision: workloads in different realms reference
        different VMs that both compute to CID ${toString collision.cid}.
        Adjust d2b.vms.<vm>.index in the affected env or rename one VM.
        Affected pairs: ${
          lib.concatStringsSep "; " (map
            (p: "${p.realmName}/${p.workloadName} -> ${p.legacyVmName}")
            collision.pairs)
        }.
      '';
    })
    crossRealmWorkloadCidCollisions
  # Cross-realm external network attachment conflict assertion.
  # Fires when two or more realms have associated envs sharing the same
  # attachment interface while runtime state is metadata-only.
  # This is advisory until realm-native networking is active.
  ++ map
    (conflict: {
      # Not a hard assertion in metadata-only runtime state: demoted to a
      # warning-like record. Flip to `assertion = false` when realm-native
      # networking advances beyond metadata-only.
      assertion = true;
      message = ''
        Advisory: d2b.realms ${
          lib.concatStringsSep ", " conflict.realmNames
        } associate with envs (${
          lib.concatStringsSep ", " conflict.envNames
        }) that share externalNetwork.attachment.interface "${conflict.interface}".
        This may conflict when realm-native networking is activated. Review
        externalNetwork.attachment settings across these realms.
      '';
    })
    crossRealmExtNetConflicts;

  autoSysVmNames =
    (lib.mapAttrsToList
      (envName: env: env.netName or "sys-${envName}-net")
      cfg.envs)
    ++ lib.optional obsCfg.enable obsCfg.vmName
    ++ autoGatewayVmNames;

  secretShaped = s:
    lib.hasInfix "SharedAccessKey" s
    || lib.hasInfix "Endpoint=sb://" s
    || lib.hasInfix "AccountKey=" s
    || lib.hasInfix "PRIVATE KEY" s
    || lib.hasInfix "BEGIN " s;
  secretLikeRef = s:
    let
      lower = lib.toLower s;
      compact = lib.replaceStrings [ "-" "_" "." " " "=" ":" "/" ] [ "" "" "" "" "" "" "" ] lower;
      markers = [
        "secret"
        "password"
        "passwd"
        "bearer"
        "private"
        "apikey"
        "sharedaccesskey"
        "accountkey"
        "endpointsb"
        "accesstoken"
        "refreshtoken"
        "sessiontoken"
        "privatekey"
        "publickey"
        "credentialmaterial"
      ];
    in
    secretShaped s || lib.any (marker: lib.hasInfix marker compact) markers;

  underStateDir = s:
    lib.hasPrefix "${toString cfg.site.stateDir}/" s;

  pathComponents = s:
    lib.filter (part: part != "") (lib.splitString "/" s);

  hasParentTraversal = s:
    builtins.elem ".." (pathComponents s);

  hasTrailingSlash = s:
    s != "/" && lib.hasSuffix "/" s;

  absoluteRuntimePathUnder = root: s:
    lib.hasPrefix "/" s
    && lib.hasPrefix "${toString root}/" s
    && !(hasParentTraversal s)
    && !(hasTrailingSlash s);

  gatewayPathAssertions =
    lib.flatten (lib.mapAttrsToList
      (name: gw:
        let
          paths = {
            stateDir = gw.stateDir;
            credentialPath = gw.credentialPath;
            sealKeyPath = gw.sealKeyPath;
          };
        in
        lib.mapAttrsToList
          (field: value: {
            assertion =
              absoluteRuntimePathUnder cfg.site.stateDir value
              && !(lib.hasPrefix "/nix/store/" value)
              && !(secretShaped value);
            message = ''
              d2b.gateways.${name}.${field} must be an absolute runtime
              path under d2b.site.stateDir, must not contain `..` path
              components or a trailing slash, and must not contain inline
              secret-shaped material. Put plaintext credentials in the gateway
              guest's sealed runtime state, not in the host Nix configuration.
            '';
          })
          paths)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways));

  gatewayCredentialPathAssertions =
    lib.flatten (lib.mapAttrsToList
      (name: gw:
        map
          (field: {
            assertion = absoluteRuntimePathUnder gw.stateDir gw.${field};
            message = ''
              d2b.gateways.${name}.${field} must live under
              d2b.gateways.${name}.stateDir so the gateway credential
              store stays inside the gateway runtime-state boundary.
            '';
          })
          [ "credentialPath" "sealKeyPath" ])
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways));

  gatewayHostRelayCredentialAssertions =
    lib.mapAttrsToList
      (name: gw: {
        assertion = !gw.allowHostRelayCredentials;
        message = ''
          d2b.gateways.${name}.allowHostRelayCredentials has been retired.
          Host-side gateway credential reads and Relay Send bearer minting are
          rejected; enroll credentials inside the gateway guest instead.
        '';
      })
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);

  realmIdentityRefFields = [
    "realmIdentityRef"
    "controllerKeyRef"
    "trustBundleRef"
    "enrollmentRef"
    "rotationPolicyRef"
  ];
  realmIdentitySecretRefRows =
    lib.flatten (map
      (realm:
        lib.filter
          (row: row.value != null && secretLikeRef row.value)
          (map
            (field: {
              inherit field;
              realmPath = realm.path;
              value = realm.keys.${field};
            })
            realmIdentityRefFields))
      enabledRealmRows);
  realmIdentitySecretRefAssertions = [
    {
      assertion = realmIdentitySecretRefRows == [ ];
      message = ''
        d2b.realms identity key refs must be opaque, non-secret locators.
        These d2b.realms.<realm>.keys fields look like inline credential or
        key material: ${
          lib.concatStringsSep ", " (map (row: "${row.realmPath}.${row.field}") realmIdentitySecretRefRows)
        }.

        Store private keys, public-key blocks, bearer tokens, passwords, and
        provider credentials outside the host Nix store; reference them with
        neutral ids such as idref-work, cgref-work, trust-work, enroll-work,
        or rotate-work.
      '';
    }
  ];

  # Validate launcher.app.targetRealm against the WorkloadTarget format:
  # <workload>.<realmPath>.d2b where every label is [a-z][a-z0-9-]*.
  # Rejects values that would fail WorkloadTarget::parse on the Rust side
  # (deny_unknown_fields / Deserialize means the bundle consumer fails on
  # a bad target rather than silently ignoring it).
  validWorkloadTarget = s:
    builtins.match "[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)+\\.d2b" s != null;
  validLauncherItemId = s:
    builtins.match "[a-z][a-z0-9-]*" s != null;
  nul = builtins.fromJSON ''"\u0000"'';

  realmWorkloadTargetAssertions =
    lib.flatten (lib.mapAttrsToList
      (realmName: realm:
        lib.mapAttrsToList
          (wName: w:
            let
              tr = w.launcher.app.targetRealm;
            in
            lib.optional
              (w.enable && tr != null && !validWorkloadTarget tr)
              {
                assertion = false;
                message = ''
                  d2b.realms.${realmName}.workloads.${wName}.launcher.app.targetRealm
                  = "${tr}" is not a valid workload target address.

                  A valid target address has the form
                  <workload>.<realmPath>.d2b where every label matches
                  [a-z][a-z0-9-]* and the last component is exactly "d2b".

                  Examples: "corp-laptop.work.d2b", "api.payments.work.d2b".
                '';
              })
          realm.workloads)
      (lib.filterAttrs (_: r: r.enable) cfg.realms));

  realmLauncherItemAssertions =
    lib.flatten (lib.mapAttrsToList
      (realmName: realm:
        lib.flatten (lib.mapAttrsToList
          (workloadName: workload:
            let
              path = "d2b.realms.${realmName}.workloads.${workloadName}";
              itemNames = lib.attrNames workload.launcher.items;
              itemRows = lib.mapAttrsToList
                (itemId: item: {
                  inherit itemId item;
                  argvBytes = lib.foldl' (total: arg: total + builtins.stringLength arg) 0 item.argv;
                })
                workload.launcher.items;
              hasExecItem = lib.any (row: row.item.type == "exec") itemRows;
              hasShellItem = lib.any (row: row.item.type == "shell") itemRows;
              unsafeLocal = workload.kind == "unsafe-local";
              localVmOptionsUnused =
                workload.localVm.memoryMiB == null
                && workload.localVm.vcpus == null
                && workload.localVm.networkIndex == null
                && workload.localVm.autostart == false
                && workload.localVm.ssh.user == null
                && workload.localVm.graphics.enable == false
                && workload.localVm.tpm.enable == false;
              qemuOptionsUnused =
                workload.qemuMedia.source == null
                && workload.qemuMedia.removableSlots == { }
                && workload.qemuMedia.bootDrive.slot == "boot"
                && workload.qemuMedia.resources.memoryMiB == 4096
                && workload.qemuMedia.resources.vcpu == 2
                && workload.qemuMedia.security.lockMemory == false
                && workload.qemuMedia.security.excludeMemoryFromCoreDump
                && workload.qemuMedia.security.disableMemoryMerge;
            in
            [
              {
                assertion = workload.launcher.defaultItem == null
                  || builtins.hasAttr workload.launcher.defaultItem workload.launcher.items;
                message = ''
                  ${path}.launcher.defaultItem must name an explicitly declared
                  launcher.items entry.
                '';
              }
              {
                assertion = !unsafeLocal || realm.policy.allowUnsafeLocal;
                message = ''
                  ${path} uses kind = "unsafe-local", but
                  d2b.realms.${realmName}.policy.allowUnsafeLocal is false.
                  Unsafe-local workloads require explicit realm opt-in because
                  they run as the authenticated host uid without isolation.
                '';
              }
              {
                assertion = !unsafeLocal || realm.allowedUsers != [ ];
                message = ''
                  ${path} uses kind = "unsafe-local", but the realm has no
                  allowedUsers. Declare at least one eligible host user.
                '';
              }
              {
                assertion = !unsafeLocal
                  || (workload.legacyVmName == null
                    && workload.stateDir == null
                    && workload.runDir == null);
                message = ''
                  ${path} is unsafe-local and must not declare legacyVmName,
                  stateDir, or runDir. The provider owns user scopes rather than
                  VM state/runtime paths.
                '';
              }
              {
                assertion = !unsafeLocal || (localVmOptionsUnused && qemuOptionsUnused);
                message = ''
                  ${path} is unsafe-local and must not configure localVm or
                  qemuMedia runtime options.
                '';
              }
              {
                assertion = !unsafeLocal
                  || (workload.launcher.app.command == null
                    && workload.launcher.actions == [ ]);
                message = ''
                  ${path} is unsafe-local and must use typed launcher.items.
                  Legacy launcher.app.command and launcher.actions shell
                  strings are not accepted for this provider.
                '';
              }
              {
                assertion = !unsafeLocal || hasExecItem || workload.shell.enable;
                message = ''
                  ${path} must declare at least one exec launcher item or enable
                  persistent shell support.
                '';
              }
              {
                assertion = builtins.length itemNames <= 64;
                message = ''
                  ${path}.launcher.items declares more than the supported
                  maximum of 64 configured items.
                '';
              }
              {
                assertion = !hasShellItem || workload.shell.enable;
                message = ''
                  ${path} declares a shell launcher item but shell.enable is
                  false.
                '';
              }
            ]
            ++ map
              (row: {
                assertion = validLauncherItemId row.itemId;
                message = "${path}.launcher.items.${row.itemId}: item ids must match ^[a-z][a-z0-9-]*$.";
              })
              itemRows
            ++ map
              (row: {
                assertion =
                  if row.item.type == "exec"
                  then row.item.argv != [ ]
                    && builtins.length row.item.argv <= 128
                    && row.argvBytes <= 16384
                    && lib.all
                      (arg:
                        builtins.stringLength arg <= 4096
                        && builtins.stringLength (builtins.replaceStrings [ nul ] [ "" ] arg)
                          == builtins.stringLength arg)
                      row.item.argv
                  else row.item.argv == [ ] && !row.item.graphical;
                message = ''
                  ${path}.launcher.items.${row.itemId} has an invalid item
                  shape. Exec argv must be non-empty, at most 128 arguments /
                  16384 bytes total / 4096 bytes per argument, and NUL-free.
                  Shell items must have empty argv and graphical = false.
                '';
              })
              itemRows)
          realm.workloads))
      (lib.filterAttrs (_: realm: realm.enable) cfg.realms));

  unsafeLocalWorkloadCountAssertions = [
    {
      assertion =
        builtins.length
          (lib.filter (row: row.kind == "unsafe-local") realmWorkloadRows)
        <= 256;
      message = ''
        d2b declares more than the supported maximum of 256 enabled
        unsafe-local workloads.
      '';
    }
  ];

  hasUnsafeLocalWorkloads =
    lib.any
      (row: row.enable && row.kind == "unsafe-local")
      realmWorkloadRows;

  sysctlInt = name:
    let value = config.boot.kernel.sysctl.${name} or 0;
    in if builtins.isInt value
       then value
       else if builtins.isString value && builtins.match "^[0-9]+$" value != null
       then lib.toInt value
       else 0;

  unsafeLocalSocketBufferAssertions = lib.optionals hasUnsafeLocalWorkloads [
    {
      assertion = sysctlInt "net.core.rmem_max" >= 524288;
      message = ''
        d2b unsafe-local requires boot.kernel.sysctl."net.core.rmem_max" to be
        at least 524288 bytes so bounded helper frames cannot fail with EMSGSIZE.
      '';
    }
    {
      assertion = sysctlInt "net.core.wmem_max" >= 524288;
      message = ''
        d2b unsafe-local requires boot.kernel.sysctl."net.core.wmem_max" to be
        at least 524288 bytes so bounded helper frames cannot fail with EMSGSIZE.
      '';
    }
  ];

  gatewayStateBoundaryAssertions =
    lib.mapAttrsToList
      (name: gw: {
        assertion =
          gw.stateDir != toString cfg.store.stateDir
          && !(underStateDir gw.stateDir && lib.hasPrefix "${toString cfg.store.stateDir}/" gw.stateDir);
        message = ''
          d2b.gateways.${name}.stateDir must not live under
          d2b.store.stateDir. Gateway credential state is distinct from
          per-VM runtime state and has different host/guest ownership.
        '';
      })
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);

  gatewayCoordinateAssertions =
    lib.flatten (lib.mapAttrsToList
      (name: gw:
        let
          coordinates = lib.filter (v: v != null) [
            gw.relay.namespace
            gw.relay.entity
            gw.aca.endpoint
            gw.aca.subscription
            gw.aca.resourceGroup
            gw.aca.sandboxGroup
            gw.aca.region
            gw.aca.diskImageId
            gw.aca.image
            gw.aca.diskName
            gw.aca.managedIdentityResourceId
            gw.display.waypipeSocket
          ];
        in
        lib.imap0
          (i: value: {
            assertion = !(secretShaped value);
            message = ''
              d2b.gateways.${name} coordinate #${toString i} looks like
              inline credential material. Gateway options may carry non-secret
              endpoint names only.
            '';
          })
          coordinates)
      (lib.filterAttrs (_: gw: gw.enable) cfg.gateways));

  gatewayEntrypointAssertions = [
    {
      assertion = !(builtins.elem "local" enabledGatewayRealms);
      message = ''
        d2b.gateways may not declare realm `local`: the local realm
        entrypoint is always host-resident so the local fast path remains
        unambiguous.
      '';
    }
    {
      assertion = duplicateGatewayRealms == [ ];
      message = ''
        d2b.gateways must declare at most one gateway-backed realm
        entrypoint per realm path. Duplicate realm path(s): ${
          lib.concatStringsSep ", " duplicateGatewayRealms
        }.
      '';
    }
    {
      assertion = duplicateGatewayVmNames == [ ];
      message = ''
        d2b.gateways must declare a separate gateway guest per
        gateway-backed realm. Duplicate gateway VM name(s): ${
          lib.concatStringsSep ", " duplicateGatewayVmNames
        }.
      '';
    }
    {
      assertion = duplicateGatewayEnvs == [ ];
      message = ''
        d2b.gateways must not place multiple gateway-backed realms on the
        same d2b.envs L2 segment. Shared gateway env(s): ${
          lib.concatStringsSep ", " duplicateGatewayEnvs
        }.
      '';
    }
  ];

  gatewayDaemonAssertions = lib.optional (enabledGatewayNames != [ ]) {
    assertion = cfg.daemonExperimental.enable;
    message = ''
      d2b.gateways requires d2b.daemonExperimental.enable = true. The
      gateway guest is supervised by the daemon control-plane package plumbing
      and has no legacy service or bash fallback.
    '';
  };

  legacyGatewayMigrationAssertions = lib.optional (cfg.gateways != { }) {
    assertion = false;
    message = ''
      d2b migration-required (legacy-surface-detected: d2b.gateways):
      `d2b.gateways` and its old gateway/ACA sandbox fields were removed as a
      public configuration surface by the realm-native cutover.

      Move non-secret coordinates into the realm-native schema, for example:

        d2b.realms.work = {
          placement = "gateway-vm";        # or provider-agent/provider-controller
          env = "work";                    # transitional link to existing d2b.envs
          providers.aca = {
            kind = "aca";
            placement = "provider-agent";
            configRef = "aca-work-non-secret-coordinates";
          };
          relay.mode = "static";
          relay.endpoints = [ "relns-example.servicebus.windows.net" ];
          relay.credentialRef = "work-relay-credential";
        };

      Do not put Relay SAS tokens, Entra tokens, sealed credential bytes, or
      ACA provider secrets in Nix. Use enrollment/import tooling when the
      realm-native runtime lands.

      This is a guarded tombstone for the transition: declaring no
      `d2b.gateways` entries keeps today's local VM behavior unchanged.
      `d2b.envs` remains the current substrate/configuration key for bridges,
      net VMs, address allocation, and workload `d2b.vms.<vm>.env`
      membership until a later realm migration explicitly replaces it.
    '';
  };

  # Systemd-escape identity regex (lower-case alnum and `-`, must
  # start with a LETTER). `^[a-z][a-z0-9-]*$` deliberately excludes
  #   * `.` (dots — systemd-escape would turn them into `\x2e`)
  #   * `_` (underscores — same)
  #   * `@` (would collide with template-instance separator)
  #   * `/` (path separator)
  #   * uppercase (NixOS option names are case-sensitive but
  #     downstream tooling like `systemctl --type=service` is not
  #     consistent; lower-case avoids the foot-gun)
  #   * leading `-` (looks like a flag)
  #   * leading digit (a numeric-prefixed VM/env name like `42web`
  #     produces unit names such as `d2b@42web.service` and tap
  #     names like `42web-l10` which are technically legal but trip up
  #     tooling that treats the leading digit as a numeric argument —
  #     e.g. `ip link show 42web-l10` resolves to the interface at
  #     index 42 first. Requiring a leading letter matches
  #     systemd-escape best practices and avoids the ambiguity.)
  vmNameOk = name:
    builtins.match "^[a-z][a-z0-9-]*$" name != null;

  # Reserved single-name: `launcher` is taken by the polkit-launcher
  # group (`d2b`) singleton. A VM named `launcher` would
  # produce `d2b-gpu` etc. users that collide with the
  # group's namespace.
  reservedVmName = name: name == "launcher";

  # Reserved prefix for auto-declared system VMs. User-declared VMs
  # cannot use this prefix because it would shadow / collide with the
  # auto-declared `sys-<env>-net` namespace.
  reservedVmPrefix = name:
    lib.hasPrefix "sys-" name && !(lib.elem name autoSysVmNames);

  # Env name regex (same shape as VM names, no `sys-` prefix
  # restriction — env names like `sys` would still be permitted by
  # this rule, but combined with the IFNAMSIZ-1 ≤ 8 char rule in
  # network.nix the practical surface is small). The check is here
  # rather than in network.nix because it's a pure naming-format
  # rule, not a topology rule.
  #
  # Same leading-letter restriction as vmNameOk
  # env names show up in interface names (`br-<env>-up`, `<env>-l1`)
  # which `ip link` and other tools treat as numeric indices when
  # they start with a digit.
  envNameOk = name:
    builtins.match "^[a-z][a-z0-9-]*$" name != null;

  obsVmDefinitions = lib.filter
    (d: builtins.isAttrs d.value && builtins.hasAttr obsCfg.vmName d.value)
    options.d2b.vms.definitionsWithLocations;

  # Pre-v0.2.0 the framework rejected ANY consumer definition under
  # `d2b.vms.<obsCfg.vmName>` to prevent "user-declared VM collides
  # with auto-declared one" mistakes. In practice that blocked
  # perfectly safe extensions like `ssh.user = "root"` on the obs
  # VM, because the framework's `observability-vm.nix` block already
  # uses `lib.mkDefault` for every value it sets — a consumer extension
  # MERGES on top of it via the module system. The assertion was
  # over-conservative and the check was removed in v0.2.0. We retain
  # `userObsVmDefinitions` purely for diagnostics in other error
  # messages elsewhere.
  userObsVmDefinitions = lib.filter
    (d: !(lib.hasSuffix "/nixos-modules/observability-vm.nix" d.file))
    obsVmDefinitions;

  vmDefinitionsFor = name:
    lib.filter
      (d: builtins.isAttrs d.value && builtins.hasAttr name d.value)
      options.d2b.vms.definitionsWithLocations;

  vmSubOptionDefined = name: optionName:
    lib.any
      (d:
        let value = d.value.${name};
        in builtins.isAttrs value && builtins.hasAttr optionName value)
      (vmDefinitionsFor name);

  vmVsockCidPairs = lib.mapAttrsToList
    (name: _vm: {
      inherit name;
      cid = config.d2b.manifest.${name}.observability.vsockCid;
    })
    (d2bLib.normalNixosVms cfg.vms);

  vmVsockCidGroups = lib.groupBy
    (pair: toString pair.cid)
    vmVsockCidPairs;

  collidingVmVsockCidGroups = lib.filterAttrs
    (_: pairs: builtins.length pairs > 1)
    vmVsockCidGroups;

  mkCidCollisionPairs = pairs:
    if pairs == [ ] then [ ] else
    let
      first = builtins.head pairs;
      rest = builtins.tail pairs;
    in
    (map (other: {
      vm1 = first.name;
      vm2 = other.name;
      cid = first.cid;
    }) rest)
    ++ mkCidCollisionPairs rest;

  vmVsockCidCollisions =
    lib.flatten (map mkCidCollisionPairs (lib.attrValues collidingVmVsockCidGroups));

  invalidVmVsockCidUsers = lib.filter
    (pair: builtins.elem pair.cid [ 0 1 2 u32Max ])
    vmVsockCidPairs;

  reservedObsCidUsers = map (pair: pair.name)
    (lib.filter
      (pair: pair.cid == obsVsockCid && !(obsCfg.enable && pair.name == obsCfg.vmName))
      vmVsockCidPairs);

  vmVsockSocketPairs = lib.mapAttrsToList
    (name: _vm: {
      inherit name;
      socket = config.d2b.manifest.${name}.observability.vsockHostSocket;
    })
    (d2bLib.normalNixosVms cfg.vms);

  socketPathTooLong = path: builtins.stringLength path > 107;

  tooLongVmVsockSockets = lib.filter
    (pair:
      socketPathTooLong pair.socket
      || socketPathTooLong "${pair.socket}_${toString d2bLib.observabilityOtlpVsockPort}"
      || socketPathTooLong "${pair.socket}_${toString d2bLib.guestControlVsockPort}")
    vmVsockSocketPairs;

  vmAssertions = lib.mapAttrsToList
    (name: vm: [
      {
        assertion = vmNameOk name;
        message = "d2b.vms.${name}: VM name must match the "
          + "regex ^[a-z][a-z0-9-]*$ (lowercase alnum + '-', "
          + "starting with a LETTER). This guarantees systemd-escape "
          + "round-trips identically, that tap/interface names "
          + "stay within IFNAMSIZ, and that tooling treating the "
          + "leading digit as a numeric index (e.g. `ip link show`) "
          + "doesn't mis-resolve the name.";
      }
      {
        assertion = !(reservedVmName name);
        message = "d2b.vms.${name}: 'launcher' is reserved for "
          + "the polkit-launcher group (d2b); pick "
          + "another name.";
      }
      {
        assertion = !(reservedVmPrefix name);
        message = "d2b.vms.${name}: names starting with 'sys-' "
          + "are reserved for d2b's auto-declared system VMs "
          + "(e.g. sys-<env>-net for each declared env, plus "
          + "d2b.observability.vmName when observability is "
          + "enabled). Rename this VM or — if it's intentionally a "
          + "system VM — register it via d2b.envs.<env>.netName "
          + "instead.";
      }
      {
        assertion = !(vm.enable && vm.observability.enable && !obsCfg.enable);
        message = "VM ${name} has observability.enable = true but d2b.observability.enable is false. Per-VM observability requires the framework-level toggle (auto-declares the sys-obs telemetry sink).";
      }
      {
        assertion = !(vm.enable && vm.audit.enable && !vm.observability.enable);
        message = "d2b.vms.${name}.audit.enable requires observability.enable on the same VM";
      }
      {
        assertion = !(vm.enable && vm.graphics.videoSidecar && !vm.graphics.enable);
        message = ''
          d2b.vms.${name}.graphics.videoSidecar requires graphics.enable = true.
          Enable graphics for this VM or disable graphics.videoSidecar.
        '';
      }
      {
        assertion = !(vm.enable && vm.graphics.videoNvidiaDecode && !vm.graphics.videoSidecar);
        message = ''
          d2b.vms.${name}.graphics.videoNvidiaDecode requires graphics.videoSidecar = true.
          Enable the video sidecar or disable graphics.videoNvidiaDecode.
        '';
      }
      {
        assertion = !(vm.enable && vm.graphics.virglVideo && !vm.graphics.enable);
        message = ''
          d2b.vms.${name}.graphics.virglVideo requires graphics.enable = true.
          Enable graphics for this VM or disable graphics.virglVideo.
        '';
      }
      {
        # Xwayland is intentionally unsupported during the Wayland-only
        # migration to wl-cross-domain-proxy + d2b-wayland-proxy.
        # wl-cross-domain-proxy is a plain cross-domain transport and
        # carries no Xwayland helper. A standalone host-side Xwayland
        # proxy is tracked as future work.
        assertion = !(vm.enable && vm.graphics.xwayland.enable);
        message = ''
          d2b.vms.${name}.graphics.xwayland.enable = true is not
          supported in this release.

          The guest Wayland transport has been replaced with
          wl-cross-domain-proxy, which does not include an Xwayland
          helper. X11 support is tracked as future work.

          Remediation: set graphics.xwayland.enable = false (the default).
        '';
      }
      {
        # primary error path (per ADR 0015): the
        # `mkRemovedOptionModule` shim approach is incompatible
        # with `attrsOf submodule` semantics (no `assertions` option
        # at the per-submodule layer). The supervisor-removal
        # friendly message is therefore emitted by this top-level
        # assertion, which fires whenever any per-VM `vm` attrset
        # carries a `supervisor` attribute.
        assertion = !(vm.enable && (vm ? supervisor));
        message = ''
          d2b.vms.${name}.supervisor was removed in v1.1
          per ADR 0015 (daemon-only clean break). The v1.0
          daemon-only end-state makes "d2bd" the only valid
          supervisor; v1.1 completes the migration by deleting
          the option entirely. Remove every "supervisor = ..."
          line from your consumer flake.

          The daemon-only path is the default and only path; see
          docs/how-to/migrate-d2b-v1-0-to-v1-1.md.
        '';
      }
      {
        # `d2b.vms.<name>.entra-id.*` was removed; the
        # option is a kept-but-internal stub so legacy assignments
        # land here instead of producing a cryptic
        # "option does not exist" error from the module system.
        assertion = vm.entra-id == { };
        message = ''
          d2b.vms.${name}.entra-id.* was removed.
          Himmelblau / Microsoft Entra ID support has moved out of
          the d2b framework into the sibling
          `vicondoa/entrablau.nix` flake. To migrate:

            inputs.entrablau.url =
              "github:vicondoa/entrablau.nix";

            d2b.vms.${name}.config.imports = [
              inputs.entrablau.nixosModules.default
            ];

            # Move each `d2b.vms.${name}.entra-id.<key>` setting
            # into the VM's guest config under the sibling module's
            # `entrablau.<key>` option tree.
            d2b.vms.${name}.config.entrablau = {
              enable    = true;
              domain    = [ "contoso.com" ];
              # ...
            };

          See CHANGELOG.md and the
          entrablau README for the full migration recipe.
        '';
      }
      {
        # `d2b.vms.<name>.guest.exec.allowRoot` was removed:
        # guest-control exec now ALWAYS runs as the VM's workload
        # user (`ssh.user`) inside a PAM login session, never root.
        # The option is a kept-but-internal stub (options-vms.nix) so
        # legacy assignments land on this friendly message instead of
        # a cryptic "option does not exist" module-system error.
        assertion = !(vm.enable && vm.guest.exec.allowRoot);
        message = ''
          d2b.vms.${name}.guest.exec.allowRoot was removed.
          Guest-control exec now always runs as the VM's workload
          user (`ssh.user`) inside a PAM login session — never as
          root. There is no root-exec mode. Remove
          `guest.exec.allowRoot = ...;`; to run a command as root,
          elevate with `sudo` inside the exec session.
        '';
      }
      {
        # `d2b.vms.<name>.guest.exec.users` was removed: there is
        # no per-VM exec user allowlist; exec always targets the
        # single workload user (`ssh.user`). Kept-but-internal stub.
        assertion = !(vm.enable && vm.guest.exec.users != [ ]);
        message = ''
          d2b.vms.${name}.guest.exec.users was removed.
          Guest-control exec now always targets the VM's single
          workload user (`ssh.user`); there is no per-VM exec user
          allowlist. Remove `guest.exec.users = [ ... ];`.
        '';
      }
      {
        assertion =
          let timeout = vm.lifecycle.gracefulShutdown.timeoutSeconds;
          in !(vm.enable && timeout != null && (timeout < 1 || timeout > 600));
        message = ''
          d2b.vms.${name}.lifecycle.gracefulShutdown.timeoutSeconds must be
          null or an integer between 1 and 600 seconds. The upper bound keeps
          host shutdown and reboot bounded; use the global
          d2b.daemon.lifecycle.gracefulShutdown.timeoutSeconds default when
          this VM does not need a different wait.
        '';
      }
      {
        assertion =
          let timeout = vm.lifecycle.liveActivation.timeoutSeconds;
          in !(vm.enable && timeout != null && (timeout < 1 || timeout > 3600));
        message = ''
          d2b.vms.${name}.lifecycle.liveActivation.timeoutSeconds must be
          null or an integer between 1 and 3600 seconds. Use the global
          d2b.daemon.lifecycle.liveActivation.timeoutSeconds default unless
          this VM legitimately waits on long in-guest activation flows.
        '';
      }
      {
        # Graphics VMs CANNOT be autostart. The
        # `d2b@<vm>` wrapper template starts `microvm@<vm>`,
        # which is the upstream microvm.nix runner — but graphics
        # VMs run cloud-hypervisor via the `d2b-<vm>-gpu`
        # sidecar (which replaces the upstream runner). The sidecar
        # binds to /run/user/<wayland-uid>/wayland-0, which only
        # exists in a live user session, so it MUST be launched
        # interactively from a Plasma terminal via `d2b up <vm>`.
        # An autostart=true graphics VM would silently boot through
        # the wrong path and never attach to the host compositor.
        assertion = !(vm.enable && vm.graphics.enable && vm.autostart);
        message = ''
          d2b.vms.${name}: graphics.enable = true is incompatible
          with autostart = true. Graphics VMs are launched by the
          d2b CLI through d2b-${name}-gpu.service, which
          binds to /run/user/<uid>/wayland-0 — that socket only
          exists in a live user session. The systemd boot path
          would start microvm@${name}.service (the upstream runner)
          bypassing the GPU sidecar entirely, and the VM would have
          no display.

          Set `d2b.vms.${name}.autostart = false` and launch
          the VM interactively via `d2b up ${name}` from a
          Plasma terminal (or wire it to your Plasma session's
          autostart entries).
        '';
      }
      {
        assertion = !(vm.enable && vm.usbip.yubikey && vm.usb.securityKey.enable);
        message = ''
          d2b.vms.${name}: usbip.yubikey = true and
          usb.securityKey.enable = true cannot both be enabled for
          the same VM. Both features claim the same FIDO2 device
          endpoint on the guest. Enable only one:
           - usbip.yubikey = true: passthrough the physical YubiKey
             USB device directly into the guest via USBIP.
           - usb.securityKey.enable = true: run the CTAPHID virtual
             UHID device frontend (connects to the host broker, does
             not require physical USB device access inside the guest).
        '';
      }
      ])
    cfg.vms;

  qemuMediaAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      let
        mediaSources =
          (lib.optional (vm.qemuMedia.source != null) {
            slot = "boot";
            source = vm.qemuMedia.source;
          })
          ++ (lib.flatten (lib.mapAttrsToList
            (slotName: slot:
              lib.optional (slot.source != null) {
                slot = slotName;
                source = slot.source;
              })
            vm.qemuMedia.removableSlots));
        declaredMediaRefs =
          map (entry: entry.source.ref)
            (lib.filter (entry: entry.source.ref != null) mediaSources);
        duplicateMediaRefs =
          lib.unique (lib.filter
            (ref: lib.length (lib.filter (other: other == ref) declaredMediaRefs) > 1)
            declaredMediaRefs);
        bootDriveSource =
          if vm.qemuMedia.bootDrive.slot == "boot" then vm.qemuMedia.source
          else (vm.qemuMedia.removableSlots.${vm.qemuMedia.bootDrive.slot}.source or null);
        sourceAssertions = lib.flatten (map
          (entry:
            let
              source = entry.source;
              sourceName = "d2b.vms.${name}.qemuMedia.${if entry.slot == "boot" then "source" else "removableSlots.${entry.slot}.source"}";
              isPhysical = source.kind == "physical-usb";
              isImage = source.kind == "image-file";
            in [
              {
                assertion = (!isPhysical) || source.ref != null;
                message = ''
                  ${sourceName}: kind = "physical-usb" requires an opaque `ref`.
                  Discover live runtime selectors with `d2b usb probe`;
                  do not place bus IDs or device paths in Nix config.
                '';
              }
              {
                assertion = (!isPhysical) || source.path == null;
                message = ''
                  ${sourceName}: kind = "physical-usb" must not set `path`.
                  Physical USB remains opaque-ref based so raw device identity
                  and paths stay out of Nix-store-backed artifacts.
                '';
              }
              {
                assertion = (!isImage) || source.path != null;
                message = ''
                  ${sourceName}: kind = "image-file" requires an absolute
                  `path`, for example
                  `/var/lib/d2b/images/${name}-${entry.slot}.img`.
                '';
              }
              {
                assertion = (!isImage) || source.format == "raw";
                message = ''
                  ${sourceName}: kind = "image-file" supports only
                  `format = "raw"`; QEMU format auto-probing is never used.
                '';
              }
              {
                assertion =
                  (!isImage)
                  || (source.path != null
                    && lib.hasPrefix "/" source.path
                    && !(lib.hasInfix "\n" source.path)
                    && !(lib.hasInfix "\r" source.path));
                message = ''
                  ${sourceName}: image-file `path` must be an absolute
                  single-line host path.
                '';
              }
            ])
          mediaSources);
      in
      lib.optionals (vm.enable && vm.runtime.kind == "qemu-media") ([
        {
          assertion = vm.env != null;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" requires
            `env` in this foundational implementation so networking can be
            derived without evaluating a guest NixOS configuration.
          '';
        }
        {
          assertion = pkgs.stdenv.hostPlatform.system == "x86_64-linux";
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is currently
            supported only on x86_64-linux because its QEMU argv uses the
            q35/virtio-vga device model.
          '';
        }
        {
          assertion = vm.qemuMedia.source != null;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" requires
            `qemuMedia.source` in this implementation. Declare either a
            physical-usb opaque ref discovered with `d2b usb probe`, or a
            direct image-file source configured in Nix.
          '';
        }
        {
          assertion = bootDriveSource != null;
          message = ''
            d2b.vms.${name}: qemuMedia.bootDrive.slot =
            "${vm.qemuMedia.bootDrive.slot}" must select `qemuMedia.source` (`boot`)
            or a removable slot with a declared source.
          '';
        }
        {
          assertion =
            bootDriveSource == null
            || bootDriveSource.kind != "physical-usb"
            || bootDriveSource.usbSelector != null;
          message = ''
            d2b.vms.${name}: physical USB boot drive
            `${vm.qemuMedia.bootDrive.slot}` requires
            `usbSelector.byIdName`. Use `d2b usb probe` to identify the
            candidate, then configure the stable `/dev/disk/by-id` basename.
          '';
        }
        {
          assertion = !(builtins.hasAttr "boot" vm.qemuMedia.removableSlots);
          message = ''
            d2b.vms.${name}: qemu-media removable slot name `boot` is
            reserved for `qemuMedia.source`. Use a different
            `qemuMedia.removableSlots.<name>` such as `backup`.
          '';
        }
        {
          assertion = duplicateMediaRefs == [ ];
          message = ''
            d2b.vms.${name}: qemu-media refs must be unique per VM;
            duplicate opaque ref(s): ${lib.concatStringsSep ", " duplicateMediaRefs}.
          '';
        }
        {
          assertion = vmSubOptionDefined name "index";
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" requires an
            explicit `index` in this foundational implementation. Set
            `d2b.vms.${name}.index` to this VM's env slot.
          '';
        }
        {
          assertion = !(vmSubOptionDefined name "config");
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" must not define
            `config`. qemu-media VMs are external media runtimes and skip the
            per-VM NixOS evaluator.
          '';
        }
        {
          assertion = vm.guestConfigFile == null;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guestConfigFile because there is no d2b-managed guest
            NixOS configuration to sync.
          '';
        }
        {
          assertion =
            !vm.guest.control.enable
            && vm.guest.control.auth.tokenFile == null
            && !vm.guest.exec.enable
            && !vm.guest.exec.allowRoot
            && vm.guest.exec.users == [ ]
            && !vm.guest.shell.enable
            && vm.guest.shell.defaultName == "default"
            && vm.guest.shell.maxSessions == 8
            && vm.guest.shell.maxAttached == 1;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest-control, guest exec, and persistent shell options.
            Disable guest.control.*, guest.exec.*, and guest.shell.* for this
            manual-only runtime.
          '';
        }
        {
          assertion =
            vm.ssh.user == null
            && vm.ssh.keyPath == null
            && vm.userAuthorizedKeys == [ ]
            && !vm.sudo;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b-managed SSH, sudo, and per-VM authorized-key options.
          '';
        }
        {
          assertion = !(vmSubOptionDefined name "homeManager");
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with home-manager guest configuration.
          '';
        }
        {
          assertion = !vm.audit.enable;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest audit forwarding.
          '';
        }
        {
          assertion = !vm.observability.enable;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with guest observability.
          '';
        }
        {
          assertion = !vm.usbip.yubikey && vm.usbip.busids == [ ];
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b USBIP/YubiKey passthrough declarations.
          '';
        }
        {
          assertion = !vm.usb.securityKey.enable;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b.vms.${name}.usb.securityKey.enable. The CTAPHID
            security-key proxy requires the Cloud Hypervisor (nixos) runtime.
          '';
        }
        {
          assertion =
            !vm.graphics.enable
            && !vm.graphics.crossDomainTrusted
            && !vm.graphics.xwayland.enable
            && !vm.graphics.videoSidecar
            && !vm.graphics.videoNvidiaDecode
            && !vm.graphics.virglVideo
            && !vm.graphics.renderNodeOnly
            && vm.graphics.niriBorderColor == null
            && !vm.graphics.waylandProxy.debugLogging
            && !vm.graphics.waylandProxy.byteLogging
            && vm.graphics.waylandProxy.denyGlobals == [ ]
            && vm.graphics.waylandProxy.allowGlobals == [ ]
            && vm.graphics.waylandProxy.maxVersions == { }
            && vm.graphics.waylandProxy.dmabufAllow == [ ]
            && vm.graphics.waylandProxy.dmabufDeny == [ ];
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b graphics options.
          '';
        }
        {
          assertion = !vm.tpm.enable;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b-managed TPM state.
          '';
        }
        {
          assertion =
            !vm.audio.enable
            && !vm.audio.allowMicByDefault
            && !vm.audio.allowSpeakerByDefault
            && vm.audio.users == [ ];
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is incompatible
            with d2b audio sidecar options.
          '';
        }
        {
          assertion = !vm.autostart;
          message = ''
            d2b.vms.${name}: runtime.kind = "qemu-media" is manual-only in
            this foundational implementation; `autostart = true` is not
            supported until unattended QMP continuation is available.
          '';
        }
      ] ++ sourceAssertions))
    cfg.vms);

  envAssertions = lib.mapAttrsToList
    (name: env:
      let
        cidr = env.uplinkSubnet;
        host = subnetIp cidr 1;
        net = subnetIp cidr 2;
      in [
        {
          assertion = envNameOk name;
          message = "d2b.envs.${name}: env name must match the "
            + "regex ^[a-z][a-z0-9-]*$ (lowercase alnum + '-', "
            + "starting with a LETTER). This guarantees systemd-escape "
            + "and `br-<env>-lan` / `<env>-l<index>` interface names "
            + "are well-formed and unambiguous to `ip link`.";
        }
        {
          assertion = cidrContains cidr host && cidrContains cidr net;
          message = "env ${name}: uplinkSubnet ${cidr} cannot be materialized — derived host IP ${host} and net IP ${net} are outside the CIDR.";
        }
      ])
    cfg.envs;

  externalNetworkAssertions = lib.flatten (lib.mapAttrsToList
    (envName: env:
      let
        externalNetwork = env.externalNetwork;
        peerEnvCidrs = lib.flatten (lib.mapAttrsToList
          (peerName: peer:
            lib.optionals (peerName != envName) [
              { inherit peerName; kind = "lanSubnet"; cidr = peer.lanSubnet; }
              { inherit peerName; kind = "uplinkSubnet"; cidr = peer.uplinkSubnet; }
            ])
          cfg._index.enabledEnvs);
        sameEnvTargets = cfg._index.workloadNamesByEnv.${envName} or [ ];
        portForwards = lib.imap0
          (i: forward: { inherit i forward; })
          externalNetwork.portForwards;
        overlapsPeer = cidr:
          map
            (peer: {
              inherit (peer) peerName kind;
              peerCidr = peer.cidr;
              inherit cidr;
            })
            (lib.filter (peer: cidrOverlaps cidr peer.cidr) peerEnvCidrs);
        egressPeerOverlaps =
          lib.concatMap overlapsPeer
            (lib.optionals externalNetwork.egress.enable externalNetwork.egress.allowedCidrs);
        portForwardPeerOverlaps = lib.flatten (map
          ({ i, forward }:
            map
              (overlap: overlap // { inherit i; })
              (lib.concatMap overlapsPeer forward.sourceCidrs))
          portForwards);
      in
      [
        {
          assertion = !(externalNetwork.attachment.enable && externalNetwork.attachment.interface == null);
          message = ''
            d2b.envs.${envName}.externalNetwork.attachment.enable requires an
            explicit d2b.envs.${envName}.externalNetwork.attachment.interface.
          '';
        }
        {
          assertion = !(externalNetwork.attachment.enable
            && externalNetwork.attachment.interface != null
            && builtins.match "^[A-Za-z0-9_-]{1,15}$" externalNetwork.attachment.interface == null);
          message = ''
            d2b.envs.${envName}.externalNetwork.attachment.interface must match
            Rust IfName syntax ^[A-Za-z0-9_-]{1,15}$ so generated
            host.json cannot pass Nix eval and fail bundle parsing later.
          '';
        }
        {
          assertion = !(externalNetwork.egress.enable && !externalNetwork.attachment.enable);
          message = ''
            d2b.envs.${envName}.externalNetwork.egress.enable requires
            d2b.envs.${envName}.externalNetwork.attachment.enable = true.
          '';
        }
        {
          assertion = !(externalNetwork.portForwards != [ ] && !externalNetwork.attachment.enable);
          message = ''
            d2b.envs.${envName}.externalNetwork.portForwards requires
            d2b.envs.${envName}.externalNetwork.attachment.enable = true.
          '';
        }
        {
          assertion = !(externalNetwork.mdns.enable && !externalNetwork.attachment.enable);
          message = ''
            d2b.envs.${envName}.externalNetwork.mdns.enable requires
            d2b.envs.${envName}.externalNetwork.attachment.enable = true.
          '';
        }
      ]
      ++ map
        ({ i, forward }: {
          assertion =
            forward.vm != null || forward.targetIp != null;
          message = ''
            d2b.envs.${envName}.externalNetwork.portForwards[${toString i}]
            must specify either vm or targetIp.
          '';
        })
        portForwards
      ++ map
        ({ i, forward }: {
          assertion =
            forward.vm == null
            || builtins.elem forward.vm sameEnvTargets;
          message = ''
            d2b.envs.${envName}.externalNetwork.portForwards[${toString i}].vm
            must name an enabled VM in the same env. Got
            `${toString forward.vm}`; valid targets: ${
              lib.concatStringsSep ", " sameEnvTargets
            }.
          '';
        })
        portForwards
      ++ map
        (overlap: {
          assertion = false;
          message = ''
            d2b.envs.${envName}.externalNetwork.egress.allowedCidrs entry
            `${overlap.cidr}` overlaps peer d2b env
            ${overlap.peerName}.${overlap.kind} (${overlap.peerCidr}).
          '';
        })
        egressPeerOverlaps
      ++ map
        (overlap: {
          assertion = false;
          message = ''
            d2b.envs.${envName}.externalNetwork.portForwards[${toString overlap.i}].sourceCidrs
            entry `${overlap.cidr}` overlaps peer d2b env
            ${overlap.peerName}.${overlap.kind} (${overlap.peerCidr}).
          '';
        })
        portForwardPeerOverlaps)
    cfg._index.enabledEnvs);

  vsockAssertions =
    map
      (collision: {
        assertion = false;
        message = "Vsock CID collision: VMs ${collision.vm1}, ${collision.vm2} both compute to CID ${toString collision.cid}. Adjust d2b.vms.<vm>.index in the affected env or rename one VM.";
      })
      vmVsockCidCollisions
    ++ map
      (pair: {
        assertion = false;
        message = ''
          d2b.vms.${pair.name}: derived Cloud Hypervisor vsock CID
          ${toString pair.cid} is reserved by Linux/AF_VSOCK. Rename the VM or
          adjust env/index so d2b derives a guest CID outside 0, 1, 2, and
          ${toString u32Max}.
        '';
      })
      invalidVmVsockCidUsers
    ++ lib.optional (obsCfg.enable && reservedObsCidUsers != [ ]) {
      assertion = false;
      message = ''
        Vsock CID 1000 is reserved for d2b.observability.vmName (${obsCfg.vmName}), but VMs ${lib.concatStringsSep ", " reservedObsCidUsers} also compute to CID 1000. Adjust d2b.vms.<vm>.index in the affected env or rename one VM.
      '';
    }
    ++ map
      (pair: {
        assertion = false;
        message = ''
          d2b.vms.${pair.name}: Cloud Hypervisor vsock socket path is too
          long for Linux AF_UNIX after port suffixes are considered:
          ${pair.socket}. Shorten d2b.site.stateDir or the VM name.
        '';
      })
      tooLongVmVsockSockets;

  # Site-level assertions (host-specific bias was extracted
  # into `d2b.site.*`; these checks make sure the consumer actually
  # set the options the framework needs for the features it enables).
  needsWaylandUser =
    lib.any
      (vm: vm.enable && (vm.graphics.enable || vm.audio.enable || vm.runtime.kind == "qemu-media"))
      (lib.attrValues cfg.vms);

  siteAssertions =
    [
      {
        assertion = toString cfg.site.stateDir == "/var/lib/d2b";
        message = ''
          d2b.site.stateDir is reserved but not fully threaded yet.
          Leave it at /var/lib/d2b for now; overriding it would
          split host-side state across inconsistent roots.
        '';
      }
      {
        assertion = toString cfg.store.stateDir == "/var/lib/d2b/vms";
        message = ''
          d2b.store.stateDir is reserved but not fully threaded yet.
          Leave it at /var/lib/d2b/vms for now; overriding it would
          desynchronise the manifest, CLI, and per-VM runtime state.
        '';
      }
    ]
    ++
    # If any VM uses graphics, audio, or qemu-media, the host MUST point
    # at a Wayland user — that's the user whose XDG_RUNTIME_DIR holds the
    # compositor / PipeWire sockets those host-side processes need.
    lib.optional needsWaylandUser {
      assertion = cfg.site.waylandUser != null;
      message = ''
        d2b: at least one declared VM has graphics.enable = true,
        audio.enable = true, or runtime.kind = "qemu-media", but
        `d2b.site.waylandUser` is unset (null). The host-side
        display/audio processes need a Wayland user so they can find the
        host compositor's pipewire-0 / wayland-0 sockets under
        /run/user/<uid>/.

        Set the option to the Plasma / sway / Hyprland user that
        invokes `d2b up <vm>`:

          d2b.site.waylandUser = "alice";

        For headless deployments with no graphics or audio VMs,
        leave the option as null and disable the offending toggles.
      '';
    }
    # If `waylandUser` is set, the corresponding system user must
    # actually exist. Otherwise the sidecar templates render with a
    # dangling /run/user/<unset-uid>/ path and the eval-time error
    # ("…has no attribute uid") is opaque.
    ++ lib.optional (cfg.site.waylandUser != null) {
      assertion = config.users.users ? "${cfg.site.waylandUser}";
      message = ''
        d2b.site.waylandUser = "${cfg.site.waylandUser}" but
        config.users.users.${cfg.site.waylandUser} is not declared.

        Declare the user in your top-level NixOS config:

          users.users.${cfg.site.waylandUser} = {
            isNormalUser = true;
            uid = 1000;            # match your real Plasma user
            extraGroups = [ "wheel" "video" "audio" ];
          };

        d2b references this user's UID to locate
        /run/user/<uid>/{wayland-0,pipewire-0} from the GPU and
        audio sidecars.
      '';
    }
    # launcherUsers entries must reference real users (same rationale
    # as waylandUser — extraGroups merging on a non-existent user is
    # a silent no-op).
    ++ map
      (u: {
        assertion = config.users.users ? "${u}";
        message = ''
          d2b.site.launcherUsers contains "${u}" but no
          users.users.${u} is declared. The d2b group
          is added to that user via extraGroups; non-existent users
          silently no-op.
        '';
      })
      cfg.site.launcherUsers;

  realmAllowedUserAssertions = lib.flatten (map
    (realm:
      map
        (u: {
          assertion =
            config.users.users ? "${u}"
            && ((config.users.users.${u}.isNormalUser or false)
              || (config.users.users.${u}.isSystemUser or false));
          message = ''
            d2b.realms.${realm.realmName}.allowedUsers contains "${u}" but no
            users.users.${u} is declared. Host-local realm socket access is
            added via extraGroups; non-existent users silently no-op.
          '';
        })
        realm.allowedUsers)
    (lib.filter (realm: realm.placement == "host-local") enabledRealmRows));

  # Validate every authorized-key entry (site-level + per-VM).
  siteAuthorizedKeyAssertions =
    validateAuthorizedKeys "d2b.site.userAuthorizedKeys"
      cfg.site.userAuthorizedKeys;

  perVmAuthorizedKeyAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      validateAuthorizedKeys
        "d2b.vms.${name}.userAuthorizedKeys"
        vm.userAuthorizedKeys)
    cfg.vms);

  volumeSerialAssertions = lib.flatten (lib.mapAttrsToList
    (name: vm:
      let
        microvm = d2bLib.vmRunner config name;
        serialIssues = volumeSerialIssues microvm.volumes;
      in
      lib.optionals (vm.enable && microvm.volumes != [ ]) [
        {
          assertion = serialIssues.duplicates == [ ];
          message = ''
            d2b.vms.${name}.config.microvm.volumes derives duplicate virtio
            disk serial(s): ${lib.concatStringsSep ", " serialIssues.duplicates}. Set explicit
            unique `serial` values on the volume entries.
          '';
        }
        {
          assertion = serialIssues.reserved == [ ];
          message = ''
            d2b.vms.${name}.config.microvm.volumes uses reserved virtio disk
            serial `rootfs`, which is owned by writableStoreOverlay. Set an
            explicit non-reserved `serial`.
          '';
        }
        {
          assertion = serialIssues.tooLong == [ ];
          message = ''
            d2b.vms.${name}.config.microvm.volumes has virtio disk serial(s)
            longer than 20 bytes: ${lib.concatStringsSep ", " serialIssues.tooLong}. Linux
            truncates virtio-blk serials, so guest mounts would not match.
          '';
        }
        {
          assertion = serialIssues.unsafe == [ ];
          message = ''
            d2b.vms.${name}.config.microvm.volumes has unsafe virtio disk
            serial(s): ${lib.concatStringsSep ", " serialIssues.unsafe}. Use
            only [A-Za-z0-9-], start with an alphanumeric character, and avoid
            delimiters such as comma, equals, slash, and control characters.
          '';
        }
      ])
    (d2bLib.normalNixosVms cfg.vms));

  # Containment for the per-VM guest-editable `guestConfigFile`: it may
  # only set guest OS options, never host-owned microvm.* / d2b.*.
  # The namespace-containment check (evalModules over the real nixpkgs
  # NixOS module set, definition-existence; catches imports / generated
  # modules / `_file` spoofing) runs in host.nix's composeVm pass and is
  # read here as `_computed.<name>.guestForbidden`. It is a policy lint,
  # not an eval-time security sandbox (see lib.nix + docs/adr/0024).
  # Only VMs that set a guestConfigFile force that per-VM evaluation, so
  # VMs without one — i.e. every existing consumer — pay nothing here.
  guestConfigContainmentAssertions = lib.mapAttrsToList
    (name: vm:
      let
        guestFile = toString vm.guestConfigFile;
        forbidden = cfg._computed.${name}.guestForbidden or [ ];
      in
      {
        assertion = forbidden == [ ];
        message = ''
          d2b.vms.${name}.guestConfigFile (${guestFile}) may only set
          guest OS options, but it (or a module it imports) sets host-owned
          option(s): ${
            lib.concatStringsSep ", " forbidden
          }. Host-owned microvm.* / d2b.* settings must live in the
          host-owned d2b.vms.${name}.config, which the guest cannot
          edit.
        '';
      })
    (lib.filterAttrs (_: vm: vm.enable && vm.guestConfigFile != null) cfg.vms);

  # ---- USB security-key proxy assertions ----------------------------
  #
  # Three properties are enforced at eval time:
  #   A. A VM that sets `usb.securityKey.enable = true` requires the
  #      host to set `d2b.host.usb.securityKey.enable = true`.
  #   B. A VM may NOT simultaneously set `usb.securityKey.enable` and
  #      `usbip.yubikey = true` (phase-1 mutual exclusion; the same
  #      physical key cannot be owned by both subsystems at once).
  #   C. Every vendorId in `d2b.host.usb.securityKey.devices` must
  #      fall within the FIDO-class vendor allowlist and device labels
  #      must be unique within the host config.
  #
  # The runtime broker adds defence-in-depth (sysfs class probing,
  # OFD lock exclusion), but the above three hold unconditionally at
  # eval time.

  hostSkEnabled = cfg.host.usb.securityKey.enable;

  # Known FIDO/CTAP-class vendor IDs (decimal). Must stay in sync with
  # options-host.nix's `knownFidoVendorIds` list.
  knownFidoVendorIds = [
    4176 2414 11415 8352 12675 1155
    9601 6724 2652 6353 4292 1254
    1267 9436
  ];

  # A — per-VM security-key requires host enable.
  securityKeyHostRequiredAssertions = lib.mapAttrsToList
    (name: vm:
      {
        assertion = !vm.enable || !vm.usb.securityKey.enable || hostSkEnabled;
        message = ''
          d2b.vms.${name}.usb.securityKey.enable = true requires
          d2b.host.usb.securityKey.enable = true.
          Set the host option to enable the security-key proxy subsystem
          before opting any VM into it.
        '';
      })
    cfg.vms;

  # B — per-VM mutual exclusion: security-key proxy and USBIP YubiKey.
  securityKeyUsbipMutualExclusionAssertions = lib.mapAttrsToList
    (name: vm:
      {
        assertion =
          !vm.enable
          || !(vm.usb.securityKey.enable && vm.usbip.yubikey);
        message = ''
          d2b.vms.${name}: usb.securityKey.enable and usbip.yubikey
          are mutually exclusive in phase 1. A VM cannot simultaneously
          use the CTAP/WebAuthn security-key proxy and the USBIP YubiKey
          passthrough for the same device. Disable one of the two options
          for VM '${name}'.
        '';
      })
    cfg.vms;

  # C — host device selector validity assertions.
  securityKeyDeviceAssertions =
    let
      devices = cfg.host.usb.securityKey.devices;

      # Uniqueness of labels.
      labels = map (d: d.label) devices;
      duplicateLabels = lib.filter
        (l: lib.count (x: x == l) labels > 1)
        labels;

      # FIDO-class vendor check.
      nonFidoDevices = lib.filter
        (d: !(lib.elem d.vendorId knownFidoVendorIds))
        devices;
    in
    lib.optionals (devices != [ ]) ([
      {
        assertion = duplicateLabels == [ ];
        message = ''
          d2b.host.usb.securityKey.devices: duplicate label(s) found:
          ${lib.concatStringsSep ", " (lib.unique duplicateLabels)}.
          Each device selector must have a unique label.
        '';
      }
      {
        assertion = nonFidoDevices == [ ];
        message = ''
          d2b.host.usb.securityKey.devices: vendorId(s) not in the
          FIDO-class allowlist: ${
            lib.concatStringsSep ", " (map (d: "0x${lib.toHexString d.vendorId} (label: ${d.label})") nonFidoDevices)
          }. Only known FIDO/CTAP security-key vendors are permitted.
          Use host udev/sysfs inventory or `d2b usb probe` to verify your
          device's vendorId, or add it to the framework allowlist if it is
          a legitimate FIDO2 device.
        '';
      }
    ]);

  deprecatedWaylandProxyBorderWarnings = lib.flatten (lib.mapAttrsToList
    (name: vm:
      let
        border = vm.graphics.waylandProxy.border;
        proxyEnabled = vm.enable && vm.graphics.enable && vm.graphics.waylandProxy.enable && border.enable;
      in
      lib.optionals (proxyEnabled && border.thickness != 9) [
        ''
          d2b.vms.${name}.graphics.waylandProxy.border.thickness is deprecated and ignored by the fixed-width wrapper rail; remove the setting.
        ''
      ]
      ++ lib.optionals (proxyEnabled && border.label.position != "top-left") [
        ''
          d2b.vms.${name}.graphics.waylandProxy.border.label.position is deprecated and ignored by the vertical wrapper rail label; remove the setting.
        ''
      ])
    cfg.vms);

  # Realm-to-legacy migration advisory warnings.
  #
  # These are informational nudges, not hard failures.  They fire
  # when a realm declaration is present but its `network.mode` or `env`
  # still points at a legacy `d2b.envs` bridge, or when a realm has no
  # workloads while one or more `d2b.vms` entries list `env` membership
  # in that realm's declared env.  The message points at the v1.2→v2
  # migration guide without blocking any activation.
  realmLegacyTransitionWarnings =
    let
      enabledRealms = lib.filterAttrs (_: r: r.enable) cfg.realms;

      # Per-realm warning: realm has env/network.envs pointing at an
      # existing d2b.envs entry but network.mode is still "none".
      inheritEnvNudges = lib.flatten (lib.mapAttrsToList
        (realmName: realm:
          let
            linkedEnvs =
              lib.unique (
                (lib.optional (realm.env != null) realm.env)
                ++ realm.network.envs
              );
            hasMatchingEnv = lib.any (e: cfg.envs ? ${e}) linkedEnvs;
          in
          lib.optionals
            (hasMatchingEnv && realm.network.mode == "none" && realm.workloads == {})
            [
              ''
                d2b.realms.${realmName}: this realm links to existing d2b.envs entries
                (${lib.concatStringsSep ", " (map (e: "d2b.envs.${e}") linkedEnvs)}) but
                has no workloads declared and network.mode = "none".

                To complete the v2 realm-native transition, migrate your env and VM
                declarations into realm workloads and set network.mode = "declared" or
                "inherit-env".  Until then the legacy substrate continues to work.

                Migration guide: docs/how-to/migrate-d2b-v1-2-to-v2.md
                Replacement surface:
                  d2b.realms.${realmName}.network (replaces d2b.envs.*)
                  d2b.realms.${realmName}.workloads (replaces d2b.vms.*)
              ''
            ])
        enabledRealms);

      # Per-realm warning: realm has workloads declared but some of them
      # carry a legacyVmName whose state dir would diverge if the VM
      # declaration is removed before checking legacyVmName.
      orphanLegacyVmWarnings = lib.flatten (lib.mapAttrsToList
        (realmName: realm:
          lib.flatten (lib.mapAttrsToList
            (wName: w:
              let
                lvm = w.legacyVmName;
                vmExists = lvm != null && cfg.vms ? ${lvm};
              in
              lib.optionals
                (w.enable && lvm != null && !vmExists)
                [
                  ''
                    d2b.realms.${realmName}.workloads.${wName}.legacyVmName = "${lvm}" but
                    d2b.vms.${lvm} does not exist in this configuration.

                    Either declare d2b.vms.${lvm} (keeping the legacy VM entry during the
                    transition) or remove legacyVmName once you no longer need the legacy
                    reference.  Workload state will still be written to the default path
                    /var/lib/d2b/vms/${wName}; mismatched legacyVmName and id can result
                    in diverged state paths.
                  ''
                ])
            realm.workloads))
        enabledRealms);
    in
    inheritEnvNudges ++ orphanLegacyVmWarnings;
in
{
  assertions = lib.flatten (
    vmAssertions
    ++ qemuMediaAssertions
    ++ envAssertions
    ++ externalNetworkAssertions
    ++ vsockAssertions
    ++ siteAssertions
    ++ realmAllowedUserAssertions
    ++ siteAuthorizedKeyAssertions
    ++ perVmAuthorizedKeyAssertions
    ++ volumeSerialAssertions
    ++ guestConfigContainmentAssertions
    ++ gatewayPathAssertions
    ++ gatewayCredentialPathAssertions
    ++ gatewayHostRelayCredentialAssertions
    ++ gatewayStateBoundaryAssertions
    ++ gatewayCoordinateAssertions
    ++ legacyGatewayMigrationAssertions
    ++ gatewayEntrypointAssertions
    ++ gatewayDaemonAssertions
    ++ realmIdentitySecretRefAssertions
    ++ realmAssertions
    ++ realmPortForwardAssertions
    ++ realmWorkloadTargetAssertions
    ++ realmLauncherItemAssertions
    ++ unsafeLocalWorkloadCountAssertions
    ++ unsafeLocalSocketBufferAssertions
    ++ securityKeyHostRequiredAssertions
    ++ securityKeyUsbipMutualExclusionAssertions
    ++ securityKeyDeviceAssertions
  );

  # The daemon-only end state is now the default. Do not warn on the
  # compatibility option here: option-default definitions make
  # `options.<path>.isDefined` true even when consumers do not set it.
  warnings = deprecatedWaylandProxyBorderWarnings ++ realmLegacyTransitionWarnings;
}
