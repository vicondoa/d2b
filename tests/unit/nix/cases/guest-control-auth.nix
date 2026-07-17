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
  "guest-control-auth/direct-consumer-safe-defaults" = {
    expr = evidence.directDefaults;
    expected = {
      credentialName = "d2b-guest-session-v2";
      credentialSourcePath =
        "/run/d2b-guest-control-host/d2b-guest-session-v2";
      workloadId = "direct-consumer";
    };
  };
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
  "guest-control-auth/runtime-wiring-is-paired" = {
    expr = evidence.pairedRuntimeWiring;
    expected = {
      dependencyBlocked = true;
      hasCanonicalWorkloadId = true;
      hasCredential = true;
      legacyTokenAbsent = true;
    };
  };
  "guest-control-auth/runtime-ancestor-traversal" = {
    expr = map
      (row: {
        inherit (row) mode grant;
        owner =
          if row.owner == "root" then "root"
          else if row.owner == evidence.authority.generation
          then "realm-controller"
          else "unexpected";
        group =
          if row.group == "d2b" then "local-lifecycle"
          else if row.group == "d2bd" then "local-controller"
          else if row.group == "d2bcg-r-${evidence.authority.realmId}"
          then "realm-internal"
          else if row.group == evidence.readerPrincipal then "gctlfs"
          else "unexpected";
        canonicalPath =
          builtins.match
            "/run/d2b(/r(/[a-z2-7]{20}(/w(/[a-z2-7]{20}(/guest-session(/d2b-guest-session-v2)?)?)?)?)?)?"
            row.path != null;
      })
      evidence.traversal;
    expected = [
      {
        canonicalPath = true;
        grant = "x";
        group = "local-lifecycle";
        mode = "1770";
        owner = "root";
      }
      {
        canonicalPath = true;
        grant = "x";
        group = "local-controller";
        mode = "0710";
        owner = "root";
      }
      {
        canonicalPath = true;
        grant = "x";
        group = "realm-internal";
        mode = "0750";
        owner = "root";
      }
      {
        canonicalPath = true;
        grant = "x";
        group = "realm-internal";
        mode = "0750";
        owner = "realm-controller";
      }
      {
        canonicalPath = true;
        grant = "x";
        group = "realm-internal";
        mode = "0750";
        owner = "realm-controller";
      }
      {
        canonicalPath = true;
        grant = "group-rx";
        group = "gctlfs";
        mode = "0750";
        owner = "root";
      }
      {
        canonicalPath = true;
        grant = "group-read";
        group = "gctlfs";
        mode = "0440";
        owner = "root";
      }
    ];
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
