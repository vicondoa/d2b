{ config, lib, generation ? 1 }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;

  # realm-observability-rows.nix emits its own path/secret rows using an
  # ad-hoc shape that predates the canonical StoragePathSpec contract.
  # Guard the import exactly like workload-process-rows.nix does so a
  # disabled observability stack never forces evaluation of the
  # normalized workload it requires.
  observabilityRows =
    if cfg.observability.enable or false
    then import ./realm-observability-rows.nix { inherit config lib; }
    else { workload.workloadId = null; paths = [ ]; };

  actor = kind: value: { inherit kind value; };
  principal = kind: value: { inherit kind value; };
  sortRows = field: lib.sort (left: right: lib.lessThan left.${field} right.${field});

  resourceFor = owner: kind: resources:
    lib.findFirst
      (resource: resource.kind == kind)
      (throw "realm storage: normalized ${owner} is missing ${kind}")
      resources;

  roleFor = kind: roles:
    lib.findFirst (role: role.roleKind == kind) null roles;

  generatedStorageId = resource:
    builtins.replaceStrings [ "/" ] [ "-" ] resource.resourceId;

  controllerActor = realmId: actor "daemon" "d2bd-r-${realmId}";
  brokerActor = realmId: actor "broker" "d2bbr-r-${realmId}";
  controllerUser = realmId: principal "user" "d2bd-r-${realmId}";
  brokerUser = realmId: principal "user" "d2bbr-r-${realmId}";
  internalGroup = realmId: principal "group" "d2bcg-r-${realmId}";

  mkPath =
    {
      id,
      scope,
      path,
      realmId,
      kind ? "directory",
      lifecycle ? "persistent",
      persistence ? "persistent",
      owner ? controllerUser realmId,
      group ? internalGroup realmId,
      mode ? "0750",
      accessAcl ? [ ],
      defaultAcl ? [ ],
      creator ? brokerActor realmId,
      writers ? [ (brokerActor realmId) ],
      readers ? [ (controllerActor realmId) (brokerActor realmId) ],
      cleanupPolicy ? "never",
      repairPolicy ? "broker-reconcile",
      restartPolicy ? "preserve-across-daemon-restart",
      adoptionPolicy ? "adopt-with-live-owner-proof",
      leaseClass ? "none",
      sensitivity ? "realm-scoped",
      noFollow ? true,
      invariants ? [ "no-symlink" "no-magic-link" "broker-opaque-id-only" "scope-authorization-required" ],
    }:
    {
      inherit
        id
        scope
        kind
        lifecycle
        persistence
        owner
        group
        mode
        accessAcl
        defaultAcl
        creator
        writers
        readers
        cleanupPolicy
        repairPolicy
        restartPolicy
        adoptionPolicy
        leaseClass
        sensitivity
        noFollow
        invariants
        ;
      pathTemplate = path;
      recursive = false;
    };

  mkConfigPath = args: mkPath (args // {
    lifecycle = "config";
    persistence = "persistent";
    owner = brokerUser args.realmId;
    mode = if (args.kind or "directory") == "regular-file" then "0640" else "0750";
    restartPolicy = "not-applicable";
    adoptionPolicy = "not-adoptable";
  });

  fdNone = {
    mechanism = "none";
    leaseTransferRecordRequired = false;
  };

  mkOfdLock =
    {
      id,
      scope,
      path,
      realmId,
      normalizedPath,
      # The storage-row id of the protected state resource this OFD lock
      # guards (e.g. the state/store-view/keys directory it serializes
      # access to) -- NOT the lock file's own storage row. Mandatory (no
      # default/null): every generated lock MUST declare exactly which
      # resource it protects so a runtime consumer never invents that
      # binding. The lock file's own storage row is resolved separately,
      # internally, by exact pathTemplate match against this lock's
      # `path`, so it needs no id here.
      resourceId,
      owner ? brokerActor realmId,
    }:
    {
      inherit id scope resourceId;
      pathTemplate = path;
      kind = "ofd";
      ownerProcess = owner;
      allowedHolders = [ owner ];
      inheritancePolicy = "close-on-exec";
      fdPassingPolicy = fdNone;
      acquireOrder = {
        scopeClass = "host";
        anchoredRoot = "state";
        inherit normalizedPath;
        lockId = id;
      };
      timeoutPolicy = {
        kind = "fail-fast";
        timeoutMs = null;
      };
      stalePolicy = {
        kind = "pidfd-proof-required";
        degradedReason = "lock-owner-ambiguous";
      };
      adoptionPolicy = "reacquire-after-proof";
      degradeScope = "realm";
      releaseAuthority = owner;
      cloexecRequired = true;
    };

  # realm-observability-rows.nix's `kind` is a purpose classification
  # ("config"/"state"/"secret-source"/"runtime"/"bounded-projection"),
  # not a StoragePathKind; map each purpose to a schema-valid kind +
  # sensitivity so the row can be re-emitted through the single
  # canonical mkPath authority instead of a second ad-hoc contract.
  observabilityKindMap = {
    config = { kind = "directory"; sensitivity = "private"; };
    state = { kind = "directory"; sensitivity = "secret-adjacent"; };
    "secret-source" = { kind = "directory"; sensitivity = "secret-adjacent"; };
    runtime = { kind = "directory"; sensitivity = "realm-scoped"; };
    "bounded-projection" = { kind = "directory"; sensitivity = "audit"; };
  };

  observabilityPathFor = row:
    let
      mapped =
        observabilityKindMap.${row.kind}
          or (throw
            "realm storage: unrecognized observability path kind '${row.kind}'");
      extraReaders = map (value: actor "external" value) (row.readers or [ ]);
    in
    mkPath {
      id = row.id;
      scope = row.scope;
      path = row.path;
      realmId = row.realmId;
      kind = mapped.kind;
      sensitivity = mapped.sensitivity;
      owner = brokerUser row.realmId;
      creator = brokerActor row.realmId;
      writers = [ (brokerActor row.realmId) ];
      readers = [ (controllerActor row.realmId) (brokerActor row.realmId) ]
        ++ extraReaders;
      mode = if mapped.kind == "regular-file" then "0640" else "0750";
      noFollow = row.noFollow or true;
    };

  storageIdsFor = workload: {
    localStateId =
      generatedStorageId
        (resourceFor "workload ${workload.workloadId}" "workload-state-data"
          workload.resources);
    diskSetId =
      generatedStorageId
        (resourceFor "workload ${workload.workloadId}" "workload-disks"
          workload.resources);
    storeViewId =
      generatedStorageId
        (resourceFor "workload ${workload.workloadId}" "workload-store-view-live"
          workload.resources);
    closureSyncId =
      generatedStorageId
        (resourceFor "workload ${workload.workloadId}" "workload-store-view-state"
          workload.resources);
    mediaSetId =
      generatedStorageId
        (resourceFor "workload ${workload.workloadId}" "workload-media"
          workload.resources);
  };

  workloadRows = realmId: realmResources: workload:
    let
      workloadId = identity.validateShortId workload.workloadId;
      scope = "workload:${workloadId}";
      normalized = kind:
        resourceFor "workload ${workloadId}" kind workload.resources;
      stateResource = normalized "workload-state";
      runResource = normalized "workload-runtime";
      configResource = normalized "workload-config";
      realmCacheResource =
        resourceFor "realm ${realmId}" "realm-cache" realmResources;
      realmAuditResource =
        resourceFor "realm ${realmId}" "realm-audit" realmResources;
      stateRoot = stateResource.path;
      runRoot = runResource.path;
      cacheRoot = "${realmCacheResource.path}/w/${workloadId}";
      configRoot = configResource.path;
      auditRoot = "${realmAuditResource.path}/w/${workloadId}";
      ids = storageIdsFor workload;
      roleActor = roleKind:
        let role = roleFor roleKind workload.roles;
        in lib.optional (role != null) (actor "role" role.roleId);
      runtimeRole =
        if (workload.spec.kind or null) == "qemu-media"
        then roleActor "qemu-media"
        else roleActor "cloud-hypervisor";
      storeRole = roleActor "virtiofsd";
      tpmRole = roleActor "swtpm";
      audioRole = roleActor "audio";
      hasGuestSessionCredential =
        lib.attrByPath
          [ "providerBindings" "runtime" "implementationId" ]
          null
          workload == "cloud-hypervisor";
      guestSessionReader =
        principal "group" "d2b-gctlfs-${workloadId}";
      guestSessionAcl = lib.optional hasGuestSessionCredential {
        principal = guestSessionReader;
        permissions = "x";
      };
      guestSessionPaths = lib.optionals hasGuestSessionCredential [
        (mkPath {
          id = "path:workload-guest-session:${workloadId}";
          inherit scope realmId;
          path = "${runRoot}/guest-session";
          lifecycle = "process-scoped";
          persistence = "process-scoped";
          owner = principal "user" "root";
          group = guestSessionReader;
          mode = "0750";
          readers = [ (brokerActor realmId) ] ++ storeRole;
          cleanupPolicy = "process-exit-with-proof";
          repairPolicy = "broker-fail-closed";
          restartPolicy = "recreate-after-owner-death";
          adoptionPolicy = "quarantine-on-ambiguity";
          leaseClass = "process-pidfd";
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = "path:workload-guest-session-credential:${workloadId}";
          inherit scope realmId;
          path = "${runRoot}/guest-session/d2b-guest-session-v2";
          kind = "regular-file";
          lifecycle = "process-scoped";
          persistence = "process-scoped";
          owner = principal "user" "root";
          group = guestSessionReader;
          mode = "0440";
          readers = [ (brokerActor realmId) ] ++ storeRole;
          cleanupPolicy = "process-exit-with-proof";
          repairPolicy = "broker-fail-closed";
          restartPolicy = "recreate-after-owner-death";
          adoptionPolicy = "quarantine-on-ambiguity";
          leaseClass = "process-pidfd";
          sensitivity = "secret-adjacent";
        })
      ];
      standard = [
        (mkConfigPath {
          id = configResource.resourceId;
          inherit scope realmId;
          path = configRoot;
        })
        (mkPath {
          id = stateResource.resourceId;
          inherit scope realmId;
          path = stateRoot;
        })
        (mkPath {
          id = (normalized "workload-state-data").resourceId;
          inherit scope realmId;
          path = (normalized "workload-state-data").path;
          writers = [ (controllerActor realmId) (brokerActor realmId) ];
        })
        (mkPath {
          id = "path:workload-state-lock:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/state/state.lock";
          kind = "regular-file";
          mode = "0600";
          owner = controllerUser realmId;
          readers = [ (controllerActor realmId) ];
        })
        (mkPath {
          id = (normalized "workload-disks").resourceId;
          inherit scope realmId;
          path = (normalized "workload-disks").path;
          mode = "0700";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ] ++ runtimeRole;
        })
        (mkPath {
          id = "path:workload-store-view:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view";
          mode = "0755";
          readers = [ (controllerActor realmId) (brokerActor realmId) ] ++ storeRole;
          invariants = [
            "no-symlink"
            "no-magic-link"
            "no-recursive-mutation"
            "same-filesystem"
            "hardlink-farm-no-recursion"
            "broker-opaque-id-only"
            "scope-authorization-required"
          ];
        })
        (mkPath {
          id = (normalized "workload-store-view-live").resourceId;
          inherit scope realmId;
          path = (normalized "workload-store-view-live").path;
          mode = "0755";
          readers = [ (controllerActor realmId) (brokerActor realmId) ] ++ storeRole;
          cleanupPolicy = "cutover-only";
          invariants = [
            "no-symlink"
            "no-magic-link"
            "no-recursive-mutation"
            "same-filesystem"
            "hardlink-farm-no-recursion"
            "broker-opaque-id-only"
            "scope-authorization-required"
          ];
        })
        (mkPath {
          id = "path:workload-store-ready:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view/live/.ready";
          kind = "regular-file";
          mode = "0444";
          readers = [ (controllerActor realmId) ] ++ storeRole;
          invariants = [
            "no-symlink"
            "same-filesystem"
            "hardlink-farm-no-recursion"
            "broker-opaque-id-only"
          ];
        })
        (mkPath {
          id = (normalized "workload-store-view-meta").resourceId;
          inherit scope realmId;
          path = (normalized "workload-store-view-meta").path;
          mode = "0755";
          readers = [ (controllerActor realmId) (brokerActor realmId) ] ++ storeRole;
          invariants = [ "no-symlink" "no-recursive-mutation" "broker-opaque-id-only" ];
        })
        (mkPath {
          id = "path:workload-store-meta-generations:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view/meta/generations";
          mode = "0755";
          readers = [ (controllerActor realmId) (brokerActor realmId) ] ++ storeRole;
          cleanupPolicy = "cutover-only";
        })
        (mkPath {
          id = (normalized "workload-store-view-state").resourceId;
          inherit scope realmId;
          path = (normalized "workload-store-view-state").path;
          mode = "0750";
          readers = [ (controllerActor realmId) (brokerActor realmId) ];
        })
        (mkPath {
          id = "path:workload-store-state-generations:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view/state/generations";
          mode = "0750";
          readers = [ (controllerActor realmId) (brokerActor realmId) ];
          cleanupPolicy = "cutover-only";
        })
        (mkPath {
          id = (normalized "workload-store-view-gcroots").resourceId;
          inherit scope realmId;
          path = (normalized "workload-store-view-gcroots").path;
          mode = "0750";
          readers = [ (controllerActor realmId) (brokerActor realmId) ];
          cleanupPolicy = "cutover-only";
          invariants = [ "no-symlink" "no-recursive-mutation" "broker-opaque-id-only" ];
        })
        (mkPath {
          id = "path:workload-store-lock:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view/sync.lock";
          kind = "regular-file";
          mode = "0600";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ];
        })
        (mkPath {
          id = (normalized "workload-tpm").resourceId;
          inherit scope realmId;
          path = (normalized "workload-tpm").path;
          mode = "0700";
          owner =
            if tpmRole == [ ] then brokerUser realmId
            else principal "role" (builtins.head tpmRole).value;
          group =
            if tpmRole == [ ] then internalGroup realmId
            else principal "role" (builtins.head tpmRole).value;
          writers = [ (brokerActor realmId) ] ++ tpmRole;
          readers = [ (brokerActor realmId) ] ++ tpmRole;
          repairPolicy = "broker-fail-closed";
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = (normalized "workload-media").resourceId;
          inherit scope realmId;
          path = (normalized "workload-media").path;
          mode = "0700";
          owner = brokerUser realmId;
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = (normalized "workload-audio").resourceId;
          inherit scope realmId;
          path = (normalized "workload-audio").path;
          writers = [ (brokerActor realmId) ] ++ audioRole;
          readers = [ (controllerActor realmId) (brokerActor realmId) ] ++ audioRole;
        })
        (mkPath {
          id = (normalized "workload-keys").resourceId;
          inherit scope realmId;
          path = (normalized "workload-keys").path;
          mode = "0700";
          owner = brokerUser realmId;
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = "path:workload-keys-lock:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/keys/keys.lock";
          kind = "regular-file";
          mode = "0600";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ];
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = "path:workload-audit:${workloadId}";
          inherit scope realmId;
          path = auditRoot;
          mode = "0750";
          owner = brokerUser realmId;
          writers = [ (brokerActor realmId) ];
          readers = [ (controllerActor realmId) (brokerActor realmId) ];
          sensitivity = "audit";
        })
        (mkPath {
          id = "path:workload-cache:${workloadId}";
          inherit scope realmId;
          path = cacheRoot;
          lifecycle = "persistent";
          persistence = "regenerable";
          cleanupPolicy = "cutover-only";
        })
        (mkPath {
          id = runResource.resourceId;
          inherit scope realmId;
          path = runRoot;
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = "process-pidfd";
          accessAcl = guestSessionAcl;
        })
      ] ++ map
        (leaf:
          let resource = normalized "workload-${leaf}";
          in mkPath {
          id = resource.resourceId;
          inherit scope realmId;
          path = resource.path;
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = if leaf == "leases" then "file-record" else "process-pidfd";
        })
        [ "sockets" "leases" ]
      ++ [
        (mkPath {
          id = "path:workload-run-roles:${workloadId}";
          inherit scope realmId;
          path = "${runRoot}/roles";
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = "process-pidfd";
        })
      ]
      ++ lib.concatMap
        (role:
          let
            resource = resourceFor "role ${role.roleId}" "role-runtime"
              (cfg._index.resources.byRoleId.${role.roleId} or [ ]);
            roleIdentity = principal "role" role.roleId;
            roleProcess = actor "role" role.roleId;
          in
          [
            (mkPath {
              id = resource.resourceId;
              inherit scope realmId;
              path = resource.path;
              lifecycle = "boot-scoped-readoptable";
              persistence = "boot-scoped";
              owner = roleIdentity;
              group = roleIdentity;
              writers = [ (brokerActor realmId) roleProcess ];
              readers = [ (controllerActor realmId) (brokerActor realmId) roleProcess ];
              cleanupPolicy = "process-exit-with-proof";
              leaseClass = "process-pidfd";
            })
          ]
          ++ lib.optionals (role.roleKind == "audio") [
            (mkPath {
              id = "path:workload-audio-mediation:${workloadId}";
              inherit scope realmId;
              path = "${resource.path}/pipewire";
              mode = "0700";
              lifecycle = "boot-scoped-readoptable";
              persistence = "boot-scoped";
              owner = roleIdentity;
              group = roleIdentity;
              writers = [ (brokerActor realmId) roleProcess ];
              readers = [ (controllerActor realmId) (brokerActor realmId) roleProcess ];
              cleanupPolicy = "process-exit-with-proof";
              leaseClass = "process-pidfd";
            })
          ])
        workload.roles
      ++ guestSessionPaths
      ++ lib.optionals (workloadId == observabilityRows.workload.workloadId)
        (map observabilityPathFor observabilityRows.paths);
      locks = [
        (mkOfdLock {
          id = "lock:workload-state:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/state/state.lock";
          normalizedPath = "r/${realmId}/w/${workloadId}/state/state.lock";
          resourceId = (normalized "workload-state-data").resourceId;
          owner = controllerActor realmId;
        })
        (mkOfdLock {
          id = "lock:workload-store:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/store-view/sync.lock";
          normalizedPath = "r/${realmId}/w/${workloadId}/store-view/sync.lock";
          resourceId = (normalized "workload-store-view-live").resourceId;
        })
        (mkOfdLock {
          id = "lock:workload-keys:${workloadId}";
          inherit scope realmId;
          path = "${stateRoot}/keys/keys.lock";
          normalizedPath = "r/${realmId}/w/${workloadId}/keys/keys.lock";
          resourceId = (normalized "workload-keys").resourceId;
        })
      ];
    in
    {
      paths = standard;
      inherit locks workloadId ids guestSessionAcl;
    };

  realmRows = realm:
    let
      realmId = identity.validateShortId realm.realmId;
      scope = "realm:${realmId}";
      realmResources = cfg._index.resources.byRealmId.${realmId} or [ ];
      normalized = kind: resourceFor "realm ${realmId}" kind realmResources;
      stateResource = normalized "realm-state";
      runResource = normalized "realm-runtime";
      cacheResource = normalized "realm-cache";
      configResource = normalized "realm-config";
      stateRoot = stateResource.path;
      runRoot = runResource.path;
      cacheRoot = cacheResource.path;
      configRoot = configResource.path;
      workloads = map (workloadRows realmId realmResources)
        (lib.filter
          (workload: (workload.spec.kind or null) != "unsafe-local")
          (cfg._index.workloads.enabledByRealmId.${realmId} or [ ]));
      guestSessionAcl =
        lib.concatMap (workload: workload.guestSessionAcl) workloads;
      storageProviderIdFor = workload:
        identity.deriveProviderId realmId "storage" "storage-${workload.workloadId}";
      storageProviderPathRows = lib.concatMap
        (workload:
          let providerId = storageProviderIdFor workload;
          in [
            (mkPath {
              id = "provider/${providerId}/state";
              inherit scope realmId;
              path = "${stateRoot}/providers/${providerId}";
            })
            (mkPath {
              id = "provider/${providerId}/runtime";
              inherit scope realmId;
              path = "${runRoot}/p/${providerId}";
              lifecycle = "boot-scoped-readoptable";
              persistence = "boot-scoped";
              cleanupPolicy = "process-exit-with-proof";
              leaseClass = "process-pidfd";
            })
          ])
        workloads;
      configuredProviderPathRows = lib.concatMap
        (provider:
          map
            (kind:
              let resource = resourceFor "provider ${provider.providerId}"
                "provider-${kind}"
                (cfg._index.resources.byProviderId.${provider.providerId} or [ ]);
              in
              mkPath ({
                id = resource.resourceId;
                inherit scope realmId;
                path = resource.path;
              } // lib.optionalAttrs (kind == "runtime") {
                lifecycle = "boot-scoped-readoptable";
                persistence = "boot-scoped";
                cleanupPolicy = "process-exit-with-proof";
                leaseClass = "process-pidfd";
              }))
            [ "state" "runtime" ])
        (lib.filter
          (provider: provider.realmId == realmId)
          cfg._index.providers.enabledList);
      paths = [
        (mkConfigPath {
          id = configResource.resourceId;
          inherit scope realmId;
          path = configRoot;
        })
      ] ++ map
        (file: mkConfigPath {
          id = "path:realm-config-${file}:${realmId}";
          inherit scope realmId;
          path = "${configRoot}/${file}.json";
          kind = "regular-file";
        })
        [ "controller" "providers" "storage" "sync" ]
      ++ [
        (mkPath {
          id = stateResource.resourceId;
          inherit scope realmId;
          path = stateRoot;
        })
        (mkPath {
          id = (normalized "realm-controller-state").resourceId;
          inherit scope realmId;
          path = (normalized "realm-controller-state").path;
          writers = [ (controllerActor realmId) (brokerActor realmId) ];
        })
        (mkPath {
          id = "path:realm-controller-lock:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/controller/state.lock";
          kind = "regular-file";
          mode = "0600";
          owner = controllerUser realmId;
          readers = [ (controllerActor realmId) ];
        })
        (mkPath {
          id = "path:realm-controller-keys:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/controller/keys";
          mode = "0700";
          owner = brokerUser realmId;
          sensitivity = "secret-adjacent";
        })
        (mkPath {
          id = (normalized "realm-broker-state").resourceId;
          inherit scope realmId;
          path = (normalized "realm-broker-state").path;
          mode = "0700";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ];
        })
        (mkPath {
          id = "path:realm-broker-lock:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/broker/state.lock";
          kind = "regular-file";
          mode = "0600";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ];
        })
        (mkPath {
          id = (normalized "realm-audit").resourceId;
          inherit scope realmId;
          path = (normalized "realm-audit").path;
          owner = brokerUser realmId;
          writers = [ (brokerActor realmId) ];
          sensitivity = "audit";
        })
        (mkPath {
          id = "path:realm-audit-lock:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/audit/audit.lock";
          kind = "regular-file";
          mode = "0600";
          owner = brokerUser realmId;
          readers = [ (brokerActor realmId) ];
          sensitivity = "audit";
        })
        (mkPath {
          id = "path:realm-audit-workloads:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/audit/w";
          owner = brokerUser realmId;
          writers = [ (brokerActor realmId) ];
          sensitivity = "audit";
        })
        (mkPath {
          id = "path:realm-providers:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/providers";
        })
        (mkPath {
          id = "path:realm-workloads:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/w";
        })
        (mkPath {
          id = cacheResource.resourceId;
          inherit scope realmId;
          path = cacheRoot;
          persistence = "regenerable";
          cleanupPolicy = "cutover-only";
        })
        (mkPath {
          id = runResource.resourceId;
          inherit scope realmId;
          path = runRoot;
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          owner = principal "user" "root";
          group = internalGroup realmId;
          accessAcl = guestSessionAcl;
          creator = actor "broker" "d2b-priv-broker";
          writers = [ (actor "broker" "d2b-priv-broker") ];
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = "process-pidfd";
        })
      ] ++ [
        (mkPath {
          id = (normalized "realm-locks").resourceId;
          inherit scope realmId;
          path = (normalized "realm-locks").path;
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = "file-record";
        })
      ] ++ map
        (leaf: mkPath {
          id = "path:realm-run-${leaf}:${realmId}";
          inherit scope realmId;
          path = "${runRoot}/${leaf}";
          lifecycle = "boot-scoped-readoptable";
          persistence = "boot-scoped";
          accessAcl = if leaf == "w" then guestSessionAcl else [ ];
          cleanupPolicy = "process-exit-with-proof";
          leaseClass = "process-pidfd";
        })
        [ "p" "w" ]
      ++ configuredProviderPathRows
      ++ storageProviderPathRows
      ++ lib.flatten (map (workload: workload.paths) workloads);
      locks = [
        (mkOfdLock {
          id = "lock:realm-controller:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/controller/state.lock";
          normalizedPath = "r/${realmId}/controller/state.lock";
          resourceId = (normalized "realm-controller-state").resourceId;
          owner = controllerActor realmId;
        })
        (mkOfdLock {
          id = "lock:realm-broker:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/broker/state.lock";
          normalizedPath = "r/${realmId}/broker/state.lock";
          resourceId = (normalized "realm-broker-state").resourceId;
        })
        (mkOfdLock {
          id = "lock:realm-audit:${realmId}";
          inherit scope realmId;
          path = "${stateRoot}/audit/audit.lock";
          normalizedPath = "r/${realmId}/audit/audit.lock";
          resourceId = (normalized "realm-audit").resourceId;
        })
      ] ++ lib.flatten (map (workload: workload.locks) workloads);
      providers = map
        (workload:
          let
            providerId = storageProviderIdFor workload;
            binding = {
              axis = "local-storage";
              inherit realmId;
              workloadId = workload.workloadId;
              inherit (workload.ids)
                localStateId
                diskSetId
                storeViewId
                closureSyncId
                mediaSetId
                ;
              resourceGeneration = generation;
            };
          in {
            descriptor = {
              schemaVersion = 2;
              inherit providerId;
              authority = { type = "storage"; };
              implementationId = "local";
              apiVersion = { major = 2; minor = 0; };
              capabilities = [
                "storage.plan"
                "storage.ensure"
                "storage.inspect"
                "storage.adopt"
                "storage.snapshot"
                "storage.destroy"
              ];
              configurationSchemaFingerprint =
                builtins.hashString "sha256" "d2b-provider-storage-local-configuration-v1";
              configuredScopeDigest = builtins.hashString "sha256" (builtins.toJSON binding);
              registryGeneration = generation;
              placement = {
                kind = "trusted-first-party-in-process";
                inherit realmId;
                controllerRole = "realm-controller";
              };
            };
            inherit binding;
          })
        workloads;
    in
    {
      inherit realmId paths locks providers guestSessionAcl;
    };

  enabledHostLocalRealms = lib.filter
    (realm: realm.placement == "host-local")
    cfg._index.realms.enabledList;
  rows = map realmRows enabledHostLocalRealms;
  allGuestSessionAcl =
    lib.concatMap (row: row.guestSessionAcl) rows;
  realmRuntimeRoot = {
    id = "path:realm-runtime-root";
    scope = "host";
    pathTemplate = "/run/d2b/r";
    kind = "directory";
    lifecycle = "boot-scoped-readoptable";
    persistence = "boot-scoped";
    owner = principal "user" "root";
    group = principal "group" "d2bd";
    mode = "0710";
    accessAcl = allGuestSessionAcl;
    defaultAcl = [ ];
    creator = actor "broker" "d2b-priv-broker";
    writers = [ (actor "broker" "d2b-priv-broker") ];
    readers = [
      (actor "daemon" "d2bd")
      (actor "broker" "d2b-priv-broker")
    ];
    cleanupPolicy = "boot";
    repairPolicy = "broker-reconcile";
    restartPolicy = "preserve-across-daemon-restart";
    adoptionPolicy = "adopt-with-live-owner-proof";
    leaseClass = "none";
    sensitivity = "realm-scoped";
    noFollow = true;
    recursive = false;
    invariants = [
      "no-symlink"
      "no-magic-link"
      "broker-opaque-id-only"
      "scope-authorization-required"
    ];
  };
in
{
  paths = sortRows "id"
    ([ realmRuntimeRoot ] ++ lib.flatten (map (row: row.paths) rows));
  locks = sortRows "id" (lib.flatten (map (row: row.locks) rows));
  providers = lib.sort
    (left: right:
      lib.lessThan left.descriptor.providerId right.descriptor.providerId)
    (lib.flatten (map (row: row.providers) rows));
}
