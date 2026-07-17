{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  workloadRows = import ./workload-process-rows.nix {
    inherit config lib;
  };
  credentialStorageFor = row:
    lib.findFirst
      (resource:
        resource.id
          == "path:workload-guest-session-credential:${row.workloadId}")
      (throw "workload ${row.workloadId} is missing guest session credential storage")
      (import ./realm-storage-rows.nix {
        inherit config lib;
      }).paths;
  rows = map
    (row:
      let
        role = lib.findFirst
          (candidate: candidate.roleKind == "virtiofsd")
          (throw "workload ${row.workloadId} is missing its virtiofsd role")
          row.roles;
        credentialStorage = credentialStorageFor row;
        readerPrincipal = "d2b-gctlfs-${row.workloadId}";
      in
      {
        inherit (row) realmId workloadId canonicalTarget;
        roleId = role.roleId;
        resourceRef = credentialStorage.id;
        format = "GuestSessionCredentialV1";
        schemaVersion = 1;
        encoding = "d2b-guest-session-v2";
        directory = builtins.dirOf credentialStorage.pathTemplate;
        target = credentialStorage.pathTemplate;
        inherit readerPrincipal;
        readerUid = d2bLib.stablePrincipalId
          readerPrincipal;
        readerGid = d2bLib.stablePrincipalId
          readerPrincipal;
        directoryOwner = "root";
        directoryGroup = readerPrincipal;
        directoryGid = d2bLib.stablePrincipalId readerPrincipal;
        directoryMode = "0750";
        owner = "root";
        group = readerPrincipal;
        gid = d2bLib.stablePrincipalId readerPrincipal;
        mode = "0440";
        guestDelivery = {
          mechanism = "systemd-load-credential";
          credentialName = "d2b-guest-session-v2";
          sourcePath =
            "/run/d2b-guest-control-host/d2b-guest-session-v2";
          owner = "root";
          group = "root";
          mode = "0400";
          ambientFallback = false;
        };
        hostMaterialization = {
          mechanism = "authenticated-component-session-fd";
          service = "d2b.broker.v2";
          method = "Apply";
          methodId = 2253834528;
          attachmentCount = 1;
          attachmentKind = "file-descriptor";
          descriptor = "memfd";
          access = "read-only";
          purpose = "request-input";
          sealedRequired = true;
          cloexecRequired = true;
          exactStorageRefRequired = true;
          pathPayloadAllowed = false;
          ambientFallback = false;
        };
        payloadContract = {
          requiredFields = [
            "sessionGeneration"
            "parentPublicKey"
            "channelBinding"
            "guestIdentity"
            "guestPublicKey"
          ];
          optionalOperationPskFields = [
            "binding"
            "secret"
          ];
          operationPskBindingFields = [
            "operationId"
            "realmId"
            "workloadId"
            "controllerGeneration"
            "workloadGeneration"
            "runtimeInstanceHandleDigest"
            "transportEndpointDigest"
            "purpose"
            "expiresAtUnixMs"
            "replayNonce"
          ];
          forbiddenFields = [
            "parentPrivateKey"
            "guestPrivateKey"
          ];
          operationPskAllOrNone = true;
          operationPskSingleUse = true;
        };
        authority = {
          generation = row.controller;
          materialization = row.broker;
          inherit (row) realmId workloadId;
        };
        lifecycle = {
          rotateOn = [
            "controller-generation"
            "workload-generation"
            "runtime-instance"
            "transport-endpoint"
            "guest-reenrollment"
          ];
          restart = "rotate-before-publish";
          adoption = "exact-binding-or-quarantine";
          stale = "fail-closed";
          ambiguous = "fail-closed";
          withdrawBootstrapPsk = true;
        };
        observability = {
          logsCredential = false;
          auditsCredential = false;
          metricsCredential = false;
          diagnostics = "closed-redacted";
        };
        runtimeDependency = {
          status = "blocked";
          codec = "GuestSessionCredentialV1";
          encoder = "realm-controller";
          decoder = "d2b-guestd";
          liveOperationRemoval = "GuestControlSign";
          requiredTogether = [
            "load-credential"
            "workload-id"
          ];
          standalone = false;
        };
        creator = row.broker;
        repairOwner = row.broker;
        materializedByHostActivation = false;
        bundleArtifact = false;
        derivationMaterial = false;
      })
    (lib.filter
      (row: row.runtimeImplementation == "cloud-hypervisor")
      workloadRows);
in
{
  options.d2b._workloadGuestSessionCredentialRows = lib.mkOption {
    type = lib.types.listOf lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._workloadGuestSessionCredentialRows = rows;
  config.users.groups = lib.listToAttrs (map
    (row: lib.nameValuePair row.readerPrincipal {
      gid = row.readerGid;
    })
    rows);
}
