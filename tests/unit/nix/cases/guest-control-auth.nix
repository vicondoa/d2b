{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  evidence = import
    (flakeRoot + "/tests/unit/nix/eval-cases/guest-control-auth-eval.nix") {
      inherit system pkgs;
      flake = flakeShim;
    };
in
{
  "guest-control-auth/realm-runtime-authority" = {
    expr = {
      inherit
        (evidence)
        authorityIsChildRealm
        bundleArtifact
        derivationMaterial
        materializedByHostActivation
        ;
    };
    expected = {
      authorityIsChildRealm = true;
      bundleArtifact = false;
      derivationMaterial = false;
      materializedByHostActivation = false;
    };
  };
  "guest-control-auth/private-credential-delivery" = {
    expr = {
      inherit
        (evidence)
        guestDelivery
        hostMaterialization
        hostStorage
        minijailReadOnly
        mountPoint
        readOnly
        sourceIsCanonicalRuntime
        ;
    };
    expected = {
      guestDelivery = {
        ambientFallback = false;
        credentialName = "d2b-guest-session-v2";
        group = "root";
        mechanism = "systemd-load-credential";
        mode = "0400";
        owner = "root";
        sourcePath =
          "/run/d2b-guest-control-host/d2b-guest-session-v2";
      };
      hostStorage = {
        directoryMode = "0750";
        groupIsWorkloadPrincipal = true;
        mode = "0440";
        owner = "root";
      };
      hostMaterialization = {
        access = "read-only";
        ambientFallback = false;
        attachmentCount = 1;
        attachmentKind = "file-descriptor";
        cloexecRequired = true;
        descriptor = "memfd";
        exactStorageRefRequired = true;
        mechanism = "authenticated-component-session-fd";
        method = "Apply";
        methodId = 2253834528;
        pathPayloadAllowed = false;
        purpose = "request-input";
        sealedRequired = true;
        service = "d2b.broker.v2";
      };
      minijailReadOnly = true;
      mountPoint = "/run/d2b-guest-control-host";
      readOnly = true;
      sourceIsCanonicalRuntime = true;
    };
  };
  "guest-control-auth/exact-payload-bindings" = {
    expr = {
      inherit (evidence) format payloadContract schemaVersion;
    };
    expected = {
      format = "GuestSessionCredentialV1";
      schemaVersion = 1;
      payloadContract = {
        forbiddenFields = [ "parentPrivateKey" "guestPrivateKey" ];
        operationPskAllOrNone = true;
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
        operationPskSingleUse = true;
        optionalOperationPskFields = [
          "binding"
          "secret"
        ];
        requiredFields = [
          "sessionGeneration"
          "parentPublicKey"
          "channelBinding"
          "guestIdentity"
          "guestPublicKey"
        ];
      };
    };
  };
  "guest-control-auth/rotation-adoption-fail-closed" = {
    expr = evidence.lifecycle;
    expected = {
      adoption = "exact-binding-or-quarantine";
      ambiguous = "fail-closed";
      restart = "rotate-before-publish";
      rotateOn = [
        "controller-generation"
        "workload-generation"
        "runtime-instance"
        "transport-endpoint"
        "guest-reenrollment"
      ];
      stale = "fail-closed";
      withdrawBootstrapPsk = true;
    };
  };
  "guest-control-auth/no-bundle-or-signing-surface" = {
    expr = {
      inherit
        (evidence)
        noCredentialInArtifacts
        noGuestControlSign
        observability
        ;
    };
    expected = {
      noCredentialInArtifacts = true;
      noGuestControlSign = true;
      observability = {
        auditsCredential = false;
        diagnostics = "closed-redacted";
        logsCredential = false;
        metricsCredential = false;
      };
    };
  };
}
