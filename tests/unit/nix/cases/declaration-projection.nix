# The declaration-derived projection on the Nix side: the packaging and
# configuration surfaces read one projected row instead of restating a
# provider's registration, its digests, or its relationships.
#
# Covers the packaging boundary: a Provider package publishes
# the projection its declaration produced, the offline catalog asserts
# agreement with it, and a Process's authored binding request compiles into the
# canonical consumer request the projection consumes.
{ mkModuleEval, lib, pkgs, ... }:

let
  mkProviderArtifact = import ../../../../nix/provider-artifact.nix {
    inherit pkgs;
  };

  digestOf = seed: "sha256:${builtins.hashString "sha256" seed}";
  fileDigest = path: "sha256:${builtins.hashFile "sha256" path}";

  executableSetDigest = digestOf "provider-declared:executable-set";
  configSchema = pkgs.writeText
    "provider-declared-config-schema.json"
    "{\"type\":\"object\"}\n";
  signature = pkgs.writeText
    "provider-declared-manifest.json.sig"
    "signature-bytes";
  publicKey = pkgs.writeText
    "provider-declared-publisher-public-key.pem"
    "publisher-public-key";
  manifest = pkgs.writeText
    "provider-declared-provider-manifest.json"
    (builtins.toJSON {
      artifactId = "provider-declared";
      digests.executable = executableSetDigest;
      trust = {
        publisher = "d2b-official";
        rootEpoch = 1;
      };
      components = [ ];
      apiBindings = [ ];
    });

  # The projection row exactly as the declaration projection renders it: the
  # digests the build produced, the Provider the declaration owns, and the
  # presentation capability each component declared with the setup
  # restrictions that capability requires.
  projectionRow = {
    providerRef = "Provider/provider-declared";
    declarationDigest = digestOf "provider-declared:declaration";
    inherit executableSetDigest;
    configDigest = fileDigest configSchema;
    components = [
      {
        componentId = "declared-service";
        presentation = "namespace-first-service-source";
        setupRestrictions = [
          "steady-state-mount-namespace"
          "zero-host-capability"
        ];
      }
    ];
  };

  mkProjection = row: {
    contractVersion = "d2b.zone.v3";
    providers = { "provider-declared" = row; };
    serviceCatalog = { };
    registrations = [ ];
    operations = [ ];
    consumerRequests = [ ];
  };

  providerPackage = pkgs.runCommand "provider-declared-package" { } ''
    mkdir -p "$out"
  '';
  # A packaged Provider that published the projection: the artifact's own
  # identity plus the presentation each component declared, and nothing the
  # catalog has to infer.
  projectedPackage = row: providerPackage // {
    passthru.providerArtifact.declaration = row // {
      artifactId = "provider-declared";
      declaredPresentations = lib.listToAttrs (map
        (component: lib.nameValuePair
          component.componentId
          component.presentation)
        row.components);
    };
  };

  guestSystemPackage = pkgs.writeText
    "declaration-projection-guest-system"
    "guest-system";
  runtimePackage = pkgs.writeText
    "declaration-projection-runtime-provider"
    "runtime-provider";
  processPackage = pkgs.writeText
    "declaration-projection-process-provider"
    "process-provider";

  zoneResources = {
    runtime = {
      type = "Provider";
      spec.artifactId = "runtime-provider";
    };
    process = {
      type = "Provider";
      spec.artifactId = "process-provider";
    };
    declared = {
      type = "Provider";
      spec.artifactId = "provider-declared";
    };
    data = {
      type = "Volume";
      spec.size = "1G";
    };
    other = {
      type = "Volume";
      spec.size = "1G";
    };
    guest = {
      type = "Guest";
      spec = {
        providerRef = "Provider/runtime";
        systemArtifactId = "guest-system";
      };
    };
    worker = {
      type = "Process";
      metadata.ownerRef = "Provider/runtime";
      spec = {
        providerRef = "Provider/process";
        executionRef = "Guest/guest";
        processClass = "service";
        template = "guest-service";
      };
    };
  };

  volumeRequest = source: access: {
    kind = "volume";
    sourceRef = source;
    slot = "root";
    view = "root";
    inherit access;
    presentation = {
      presentation = "filesystem";
      destination = "/data";
    };
  };

  base = { ... }: {
    d2b.artifacts = {
      guest-system = {
        package = guestSystemPackage;
        type = "nixos-system";
      };
      runtime-provider = {
        package = runtimePackage;
        type = "provider";
      };
      process-provider = {
        package = processPackage;
        type = "provider";
      };
      provider-declared = {
        package = providerPackage;
        type = "provider";
      };
    };
    d2b.zones.alpha.resources = zoneResources;
    d2b.guestSystems.alpha.guest = {
      config.system.build.toplevel = guestSystemPackage;
    };
  };

  assertionsModule = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [ ];
    };
  };

  evalFixture = extra: (mkModuleEval [ assertionsModule base extra ]).config;

  failures = extra:
    map (assertion: assertion.message) (lib.filter
      (assertion: !assertion.assertion)
      (evalFixture extra).assertions);
  # The refusals this layer owns, isolated from the Zone-wide ResourceRef
  # checks the same fixture trips on its own.
  bindingRequestFailures = extra:
    lib.filter
      (message:
        lib.hasInfix "bindingRequests" message
        || lib.hasInfix "claim one consumer slot" message)
      (failures extra);
  # The catalog-agreement refusals, isolated from whatever else a deliberately
  # minimal fixture may legitimately trip.
  disagreements = extra:
    lib.filter (message: lib.hasInfix "disagree" message) (failures extra);

  # The authored shorthand is a list, so one declaration can be written in
  # either order; the compiled rows cannot be.
  requestsInAuthoredOrder = {
    d2b.zones.alpha.resources.worker.spec.bindingRequests = [
      (volumeRequest "Volume/data" "read-write")
      (volumeRequest "Volume/other" "read-only" // { slot = "spare"; })
    ];
  };
  requestsInReversedOrder = {
    d2b.zones.alpha.resources.worker.spec.bindingRequests = [
      (volumeRequest "Volume/other" "read-only" // { slot = "spare"; })
      (volumeRequest "Volume/data" "read-write")
    ];
  };
  compiledRequests = extra: (evalFixture extra).d2b._bundle.consumerRequests;
  processSpecInBundle = extra:
    let
      resources = (evalFixture extra).d2b._bundle.zoneResourceBundlesV3.alpha.data.resources;
      worker = lib.findFirst (resource: resource.type == "Process") null resources;
    in
    worker.spec;
in
{
  # The authored shorthand compiles into the canonical consumer
  # request the projection consumes - the projection's own Nix view, in slot
  # order, byte-identical whichever order the declaration was written in.
  "declaration-projection/consumer-request-shorthand-is-canonical-and-byte-stable" =
    let
      authored = compiledRequests requestsInAuthoredOrder;
      reversed = compiledRequests requestsInReversedOrder;
      rows = authored.requests;
    in {
      expr = {
        orderIndependent = authored == reversed;
        digestStable = authored.digest == reversed.digest;
        rowShape = lib.sort lib.lessThan (builtins.attrNames (builtins.head rows));
        rows = map (row: {
          zone = row.zone;
          consumerRef = row.consumerRef;
          kind = row.kind;
          slot = row.slot;
          fingerprintIsCanonical =
            builtins.match "sha256:[0-9a-f]{64}" row.fingerprint != null;
        }) rows;
        # The shorthand is not a second relationship list: it compiles away
        # before the row is projected.
        shorthandAbsentFromBundle =
          !(builtins.hasAttr "bindingRequests"
            (processSpecInBundle requestsInAuthoredOrder));
      };
      expected = {
        orderIndependent = true;
        digestStable = true;
        rowShape = [
          "consumerRef"
          "fingerprint"
          "kind"
          "slot"
          "zone"
        ];
        rows = [
          {
            zone = "alpha";
            consumerRef = "Process/worker";
            kind = "volume";
            slot = "root";
            fingerprintIsCanonical = true;
          }
          {
            zone = "alpha";
            consumerRef = "Process/worker";
            kind = "volume";
            slot = "spare";
            fingerprintIsCanonical = true;
          }
        ];
        shorthandAbsentFromBundle = true;
      };
    };

  # Scenario 2: two different requests claiming one consumer slot are refused
  # at eval time, and the same slot declared twice with an identical payload
  # coalesces instead.
  "declaration-projection/duplicate-consumer-slot-refuses" = {
    expr = {
      conflicting = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Volume/data" "read-write")
          (volumeRequest "Volume/other" "read-write")
        ];
      };
      coalesced = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Volume/data" "read-write")
          (volumeRequest "Volume/data" "read-write")
        ];
      };
    };
    expected = {
      conflicting = [
        "Two different canonical consumer requests claim one consumer slot:\nalpha/Process/worker#volume:root. A slot is claimed once\nand an identical repeat coalesces.\n"
      ];
      coalesced = [ ];
    };
  };

  # Scenario 2: the source policy refuses a request whose source is not a
  # Volume declared in the same Zone, a request naming a kind this layer does
  # not compile, and a request on a consumer whose canonical projection it
  # does not own.
  "declaration-projection/binding-request-outside-the-source-policy-refuses" = {
    expr = {
      missingSource = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Volume/absent" "read-write")
        ];
      };
      foreignSourceType = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Provider/runtime" "read-write")
        ];
      };
      uncompiledKind = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Volume/data" "read-write" // { kind = "network"; })
        ];
      };
      uncompiledPresentation = bindingRequestFailures {
        d2b.zones.alpha.resources.worker.spec.bindingRequests = [
          (volumeRequest "Volume/data" "read-write" // {
            presentation = {
              presentation = "filesystem";
              destination = "relative/path";
            };
          })
        ];
      };
      foreignConsumer = bindingRequestFailures {
        d2b.zones.alpha.resources.guest.spec.bindingRequests = [
          (volumeRequest "Volume/data" "read-write")
        ];
      };
    };
    expected = {
      missingSource = [
        "d2b.zones.alpha.resources.worker.spec.bindingRequests.0.sourceRef must resolve to a Volume declared in the same Zone."
      ];
      foreignSourceType = [
        "d2b.zones.alpha.resources.worker.spec.bindingRequests.0.sourceRef must resolve to a Volume declared in the same Zone."
      ];
      uncompiledKind = [
        "d2b.zones.alpha.resources.worker.spec.bindingRequests.0.kind must be one of\nvolume; another binding kind's\ndesired schema is compiled by the projection, not restated here.\n"
      ];
      uncompiledPresentation = [
        "d2b.zones.alpha.resources.worker.spec.bindingRequests.0.presentation must be an absolute filesystem destination or a bounded consumer device slot."
      ];
      foreignConsumer = [
        "d2b.zones.alpha.resources.guest.spec.bindingRequests is\ncompiled only for a Process or EphemeralProcess\nconsumer; a Guest row is projected\nelsewhere, so its request would never be compiled.\n"
      ];
    };
  };

"declaration-projection/presentation-capability-is-the-projected-one" =
    let
      packaged = mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection projectionRow;
      };
      restated = mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection (projectionRow // {
          components = [
            {
              componentId = "declared-service";
              presentation = "filesystem-presentation";
              setupRestrictions = [ "mount-tree-before-user-namespace" ];
            }
          ];
        });
      };
      presentationsOf = artifact:
        artifact.package.passthru.providerArtifact.declaration.declaredPresentations;
    in {
      expr = {
        presentation = presentationsOf packaged;
        publisherRef = packaged.trustedPublisher.publisherRef;
        signingKeyComesFromThePublisher =
          packaged.trustedPublisher.signingKey == "publisher-public-key";
        # The capability is read, never inferred: a declaration that projects
        # a different capability moves what this surface sees.
        movedWithTheDeclaration =
          presentationsOf restated != presentationsOf packaged;
      };
      expected = {
        presentation = {
          declared-service = "namespace-first-service-source";
        };
        publisherRef = "d2b-official";
        signingKeyComesFromThePublisher = true;
        movedWithTheDeclaration = true;
      };
    };

  # The exact artifact digest verification is unchanged: a build whose
  # executable set or configuration schema is not the one the declaration
  # pins is refused, and a projection that produces no row for this artifact
  # is refused rather than signed around.
  "declaration-projection/executable-digest-mismatch-refuses" = {
    expr = {
      mismatchedExecutableSet = builtins.tryEval (mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection (projectionRow // {
          executableSetDigest = digestOf "provider-declared:other-executable-set";
        });
      }).catalog;
      mismatchedConfigSchema = builtins.tryEval (mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection (projectionRow // {
          configDigest = digestOf "provider-declared:other-schema";
        });
      }).catalog;
      missingRow = builtins.tryEval (mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = { contractVersion = "d2b.zone.v3"; };
      }).catalog;
    };
    expected = {
      mismatchedExecutableSet = {
        success = false;
        value = false;
      };
      mismatchedConfigSchema = {
        success = false;
        value = false;
      };
      missingRow = {
        success = false;
        value = false;
      };
    };
  };

  # The projection is data with a closed shape: a signing key, a seccomp
  # label, or a serving-worker role cannot ride on it into packaging.
  "declaration-projection/declaration-projection-carries-no-signing-or-inference-field" = {
    expr = {
      signingKey = builtins.tryEval (mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection (projectionRow // {
          signingKey = "not-a-key";
        });
      }).catalog;
      workerRole = builtins.tryEval (mkProviderArtifact {
        artifactId = "provider-declared";
        binary = providerPackage;
        binaryRef = "provider-declared";
        inherit manifest signature configSchema publicKey;
        declarationProjection = mkProjection (projectionRow // {
          components = [
            ((builtins.head projectionRow.components) // {
              seccompLabel = "unconfined";
              workerRole = "root";
            })
          ];
        });
      }).catalog;
    };
    expected = {
      signingKey = {
        success = false;
        value = false;
      };
      workerRole = {
        success = false;
        value = false;
      };
    };
  };

  # The offline catalog and the declaration projection must agree on exactly
  # which Providers exist, in both directions. The comparison is closed and
  # always runs: a selected row the declaration does not produce and a
  # produced declaration no row selects are both refused, while a
  # configuration that selects nothing and produces nothing satisfies the
  # same comparison truthfully because both closed sets are empty. An empty
  # declaration set behind a selected row is therefore a named refusal, not
  # a quiet pass.
  "declaration-projection/catalog-agreement-refuses-a-row-no-declaration-produces" = {
    expr = {
      nothingSelectedNothingDeclared = disagreements { } == [ ];
      agreed = disagreements ({
        d2b.artifacts.provider-declared.package =
          lib.mkForce (projectedPackage projectionRow);
        d2b.providerCatalog.declared.artifactId = "provider-declared";
      }) == [ ];
      selectedWithoutDeclaration = lib.length (disagreements {
        d2b.providerCatalog.runtime.artifactId = "runtime-provider";
        d2b.artifacts.provider-declared.package =
          lib.mkForce (projectedPackage projectionRow);
      });
      declaredWithoutSelection = lib.length (disagreements {
        d2b.artifacts.provider-declared.package =
          lib.mkForce (projectedPackage projectionRow);
      });
      # Each refusal states the expectation it failed: the rows the catalog
      # selected and the rows the declarations produced. Joined rather than
      # headed, so an absent refusal reads as a false expectation instead of
      # an out-of-bounds crash.
      selectedNamesBothSets =
        let
          stated = builtins.concatStringsSep " " (disagreements {
            d2b.providerCatalog.runtime.artifactId = "runtime-provider";
          });
        in
          lib.hasInfix "The catalog selects [ runtime-provider ];" stated
          && lib.hasInfix "the declarations produce [ ]." stated;
      declaredNamesBothSets =
        let
          stated = builtins.concatStringsSep " " (disagreements {
            d2b.artifacts.provider-declared.package =
              lib.mkForce (projectedPackage projectionRow);
          });
        in
          lib.hasInfix "The catalog selects [ ];" stated
          && lib.hasInfix
            "the declarations produce [ provider-declared ]." stated;
    };
    expected = {
      nothingSelectedNothingDeclared = true;
      agreed = true;
      selectedWithoutDeclaration = 1;
      declaredWithoutSelection = 1;
      selectedNamesBothSets = true;
      declaredNamesBothSets = true;
    };
  };
}
