{ config, lib, pkgs }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;
  vhostDeviceSound = import ../pkgs/vhost-device-sound { inherit pkgs; };

  sortBy = field:
    lib.sort (left: right: lib.lessThan left.${field} right.${field});
  attrPathOr = path: fallback: attrs:
    lib.attrByPath path fallback attrs;

  audioRoles = lib.filter
    (role:
      role.enabled
      && role.roleKind == "audio"
      && (cfg._index.realms.byId.${role.realmId}.placement or null)
        == "host-local")
    cfg._index.roles.list;

  rowFor = audioRole:
    let
      realmId = identity.validateShortId audioRole.realmId;
      workloadId = identity.validateShortId audioRole.workloadId;
      roleId = identity.validateShortId audioRole.roleId;
      workload = cfg._index.workloads.byId.${workloadId};
      runtimeBinding = workload.providerBindings.runtime or null;
      audioBinding = workload.providerBindings.audio or null;
      runtimeProvider =
        if runtimeBinding == null then
          null
        else
          cfg._index.providers.byId.${runtimeBinding.providerId} or
            (throw "realm audio workload ${workloadId} references an unknown runtime provider");
      audioProvider =
        if audioBinding == null then
          null
        else
          cfg._index.providers.byId.${audioBinding.providerId} or
            (throw "realm audio workload ${workloadId} references an unknown audio provider");
      workloadRoles = cfg._index.roles.byWorkloadId.${workloadId} or [ ];
      runtimeRole = lib.findFirst
        (role: role.enabled && role.roleKind == "cloud-hypervisor")
        null
        workloadRoles;
      runtimeRoleId =
        if runtimeRole == null then null
        else identity.validateShortId runtimeRole.roleId;
      audio = workload.spec.audio or { };

      stateRoot = "/var/lib/d2b/r/${realmId}/w/${workloadId}/audio";
      runRoot = "/run/d2b/r/${realmId}/w/${workloadId}";
      roleRoot = "${runRoot}/roles/${roleId}";
      socketPath = "${runRoot}/sockets/audio.sock";
      mediationRoot = "${roleRoot}/pipewire";
      runtimeBinary = "${roleRoot}/d2b-audio-${workloadId}";

      processId = "audio-process-${roleId}";
      endpointId = "audio-vhost-${workloadId}";
      stateStorageId = "audio-state-${workloadId}";
      lockStorageId = "audio-lock-${workloadId}";
      mediationStorageId = "audio-runtime-${roleId}";
      leaseId = "audio-pipewire-${workloadId}";
      normalizedAuthorityError =
        if runtimeBinding == null then
          "realm audio workload ${workloadId} has no normalized runtime provider binding"
        else if audioBinding == null then
          "realm audio workload ${workloadId} has no normalized audio provider binding"
        else if !(
          runtimeBinding.providerType == "runtime"
          && runtimeBinding.implementationId == "cloud-hypervisor"
          && runtimeProvider.enabled
          && runtimeProvider.providerType == "runtime"
          && runtimeProvider.realmId == realmId
          && runtimeProvider.implementationId == "cloud-hypervisor"
          && runtimeProvider.placement == "host-local"
          && audioBinding.providerType == "audio"
          && audioBinding.implementationId == "pipewire-vhost-user"
          && audioProvider.enabled
          && audioProvider.providerType == "audio"
          && audioProvider.realmId == realmId
          && audioProvider.implementationId == "pipewire-vhost-user"
          && audioProvider.placement == "host-local"
        ) then
          "realm audio workload ${workloadId} provider bindings disagree with normalized authority"
        else
          null;
    in
    if normalizedAuthorityError != null then
      throw normalizedAuthorityError
    else {
      inherit
        realmId
        workloadId
        roleId
        runtimeRoleId
        processId
        endpointId
        stateStorageId
        lockStorageId
        mediationStorageId
        leaseId
        ;

      process = {
        inherit processId realmId workloadId roleId;
        kind = "vhost-user-sound";
        executable = {
          immutableSource = "${vhostDeviceSound}/bin/vhost-device-sound";
          runtimePath = runtimeBinary;
          installation = "broker-copy-immutable";
        };
        argv = [
          runtimeBinary
          "--socket"
          socketPath
          "--backend"
          "pipewire"
        ];
        environment = [
          "PIPEWIRE_RUNTIME_DIR=${mediationRoot}"
          "XDG_RUNTIME_DIR=${mediationRoot}"
          ''PIPEWIRE_PROPS={ application.name = "d2b-${workloadId}" node.name = "d2b-${workloadId}" node.description = "d2b ${workloadId}" }''
        ];
        dynamicPipeWireProperties = {
          stateStorageRef = stateStorageId;
          properties = {
            mic = "d2b.mic";
            speaker = "d2b.speaker";
          };
          invalidStatePolicy = "both-off";
        };
        readiness = {
          kind = "unix-socket-listening";
          endpointRef = endpointId;
        };
        startAfterLeaseIds = [ leaseId ];
        startBeforeRoleIds = lib.optional (runtimeRoleId != null) runtimeRoleId;
        restartPolicy = "workload-cycle-only";
        supervision = "realm-controller-pidfd";
        cgroupPlacement = "direct-role-leaf";
        seccompPolicyRef = "w1-audio";
      };

      endpoint = {
        inherit endpointId realmId workloadId roleId;
        kind = "vhost-user-sound";
        transport = "unix-stream";
        path = socketPath;
        mode = "0660";
        ownerRoleId = roleId;
        peerRoleIds = lib.optional (runtimeRoleId != null) runtimeRoleId;
        lifecycle = "workload";
        listenerOwner = "audio-role";
      };

      storage = [
        {
          storageId = stateStorageId;
          inherit realmId workloadId roleId;
          kind = "audio-policy-state";
          path = "${stateRoot}/audio-state.json";
          mode = "0640";
          lifecycle = "persistent";
          repairOwner = "realm-broker";
          mutationOwner = "audio-provider";
          atomicReplace = true;
          maxBytes = 128;
          initialState = {
            mic =
              if attrPathOr [ "allowMicByDefault" ] false audio
              then "on"
              else "off";
            speaker =
              if attrPathOr [ "allowSpeakerByDefault" ] false audio
              then "on"
              else "off";
          };
        }
        {
          storageId = lockStorageId;
          inherit realmId workloadId roleId;
          kind = "audio-policy-ofd-lock";
          path = "${runRoot}/leases/audio.lock";
          mode = "0600";
          lifecycle = "boot-scoped-readoptable";
          repairOwner = "realm-broker";
          lock = {
            kind = "ofd";
            cloexecRequired = true;
            adoptionPolicy = "reacquire-after-proof";
          };
        }
        {
          storageId = mediationStorageId;
          inherit realmId workloadId roleId;
          kind = "audio-mediation-runtime";
          path = mediationRoot;
          mode = "0700";
          lifecycle = "process";
          repairOwner = "realm-broker";
          cleanupPolicy = "process-exit-with-proof";
        }
      ];

      lease = {
        inherit leaseId realmId workloadId roleId;
        kind = "pipewire-session-endpoint";
        share = "shared-partition";
        source = {
          kind = "active-host-audio-session";
          refName = "pipewire";
        };
        delivery = {
          kind = "bind-single-endpoint";
          targetStorageRef = mediationStorageId;
          targetRelativePath = "pipewire-0";
          parentRuntimeVisible = false;
        };
        acquisitionOrder = {
          phase = 45;
          ordinal = 0;
        };
        revocation = "workload-stop";
      };

      assertions = [
        {
          assertion = runtimeRoleId != null;
          message = "realm audio requires a cloud-hypervisor role in workload ${workloadId}.";
        }
        {
          assertion = !(attrPathOr [ "autostart" ] false workload.spec);
          message = "realm audio workload ${workloadId} must be interactively started.";
        }
      ];
    };

  rows = map rowFor audioRoles;
in
{
  schemaVersion = "v2";
  workloads = rows;
  processes = sortBy "processId" (map (row: row.process) rows);
  endpoints = sortBy "endpointId" (map (row: row.endpoint) rows);
  storage = sortBy "storageId" (lib.flatten (map (row: row.storage) rows));
  leases = sortBy "leaseId" (map (row: row.lease) rows);
  assertions = lib.flatten (map (row: row.assertions) rows);
}
