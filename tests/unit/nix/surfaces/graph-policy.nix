# The isolated new-graph Nix test surface.
#
# Canonical graph policy, or a named refusal. Every case evaluates the
# test-owned projection in `tests/unit/nix/graph-policy.nix` over a
# declaration projection document and asserts what comes out: the exact
# canonical policy for a well-formed projection, and the exact refusal naming
# the retired knob or the undeclared row for a projection that still speaks
# the old vocabulary.
#
# The surface imports no production option module and declares no NixOS
# option, so nothing here can change what `nixosModules.default` evaluates
# to. That is the point of staging the new graph's Nix half as an isolated
# derived test artifact rather than by editing `nixos-modules/`.
{ flakeRoot, pkgs, ... }:

let
  inherit (pkgs) lib;
  compile = import (flakeRoot + "/tests/unit/nix/graph-policy.nix") { inherit lib; };

  # The canonical policy a projection earns when it earns no refusal.
  policy = args: (compile args).policy;
  # The refusal a projection earns, named rather than merely observed as a
  # throw: `refused` proves the policy really is refused, and `refusals` is the
  # reason it is refused. Asserting only the throw lets a case pass for an
  # unrelated defect, which is how a case named for a retired knob survives
  # the knob it names being gone.
  refusal = args: {
    refused = builtins.tryEval (policy args);
    refusals = (compile args).refusals;
  };

  digest = seed: "sha256:${builtins.hashString "sha256" seed}";

  component = {
    componentId = "declared-service";
    presentation = "namespace-first-service-source";
    setupRestrictions = [
      "steady-state-mount-namespace"
      "zero-host-capability"
    ];
  };

  # The projection exactly as the declaration projection renders it: one
  # Provider, the component its declaration carried, the method that
  # component answers, the service that method is served under, and the
  # consumer request a binding slot compiles to.
  canonical = {
    contractVersion = "d2b.zone.v3";
    providers."provider-declared" = {
      providerRef = "Provider/provider-declared";
      declarationDigest = digest "provider-declared:declaration";
      executableSetDigest = digest "provider-declared:executable-set";
      configDigest = digest "provider-declared:config-schema";
      components = [ component ];
    };
    serviceCatalog."declared.d2bus.org/export" = "Provider/provider-declared";
    registrations = [
      {
        artifactId = "provider-declared";
        providerRef = "Provider/provider-declared";
        services = [ "declared.d2bus.org/export" ];
      }
    ];
    operations = [
      {
        providerRef = "Provider/provider-declared";
        componentId = "declared-service";
        method = "export";
        presentation = "namespace-first-service-source";
      }
    ];
    consumerRequests = [
      {
        zone = "work";
        consumerRef = "Provider/runtime";
        slot = "export";
        kind = "volume";
        fingerprint = digest "provider-declared:consumer-request";
      }
    ];
  };

  expected = {
    contractVersion = "d2b.zone.v3";
    providerRefs = [ "Provider/provider-declared" ];
    components = [
      {
        componentId = "declared-service";
        presentation = "namespace-first-service-source";
        setupRestrictions = component.setupRestrictions;
        providerRef = "Provider/provider-declared";
        declarationDigest = digest "provider-declared:declaration";
      }
    ];
    methods = canonical.operations;
    consumerRequests = canonical.consumerRequests;
  };

  # A projection carrying one extra key, so a row that smuggles a retired
  # table back in is refused rather than read as if the key were absent.
  withRetiredKey = canonical // {
    providers."provider-declared" = canonical.providers."provider-declared" // {
      privileges = {
        read = [ "global" ];
        write = [ ];
      };
    };
  };
in
{
  cases = {
    "graph-policy/a-declaration-projection-compiles-the-canonical-policy" = {
      expr = (policy { projection = canonical; }).methods;
      expected = expected.methods;
    };

    "graph-policy/the-canonical-policy-is-the-whole-projection" = {
      expr = policy { projection = canonical; };
      expected = expected;
    };

    "graph-policy/a-retired-privilege-knob-is-refused" = {
      expr = refusal {
        projection = canonical;
        legacy.privileges = { read = [ "global" ]; };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "refused retired knob(s) privileges; the canonical graph policy is derived from the provider declaration and admits no second source"
        ];
      };
    };

    "graph-policy/a-retired-role-scope-table-is-refused" = {
      expr = refusal {
        projection = canonical;
        legacy.roleScopes."HostReconcile" = {
          principals = [ "d2bd" ];
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "refused retired knob(s) roleScopes; the canonical graph policy is derived from the provider declaration and admits no second source"
        ];
      };
    };

    "graph-policy/a-retired-family-scope-table-is-refused" = {
      expr = refusal {
        projection = canonical;
        legacy.familyScopes.volume = {
          storageRoots = [ "/var/lib/d2b" ];
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "refused retired knob(s) familyScopes; the canonical graph policy is derived from the provider declaration and admits no second source"
        ];
      };
    };

    "graph-policy/a-retired-broker-wire-variant-list-is-refused" = {
      expr = refusal {
        projection = canonical;
        legacy.brokerOperations = [ "ExportVolume" ];
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "refused retired knob(s) brokerOperations; the canonical graph policy is derived from the provider declaration and admits no second source"
        ];
      };
    };

    "graph-policy/a-retired-principal-allocation-is-refused" = {
      expr = refusal {
        projection = canonical;
        legacy.principalAllocation = {
          d2bd = "d2bd.d2bus.org";
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "refused retired knob(s) principalAllocation; the canonical graph policy is derived from the provider declaration and admits no second source"
        ];
      };
    };

    "graph-policy/a-projection-row-carrying-a-retired-table-is-refused" = {
      expr = refusal { projection = withRetiredKey; };
      expected = {
        refused = { success = false; value = false; };
        refusals = [ "provider provider-declared carries privileges" ];
      };
    };

    "graph-policy/a-method-no-declared-component-answers-is-refused" = {
      expr = refusal {
        projection = canonical // {
          operations = [
            {
              providerRef = "Provider/provider-declared";
              componentId = "undeclared-worker";
              method = "import";
              presentation = "none";
            }
          ];
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "method import is answered by undeclared-worker with presentation none, which Provider/provider-declared does not declare"
        ];
      };
    };

    "graph-policy/a-method-restating-another-presentation-is-refused" = {
      expr = refusal {
        projection = canonical // {
          operations = [
            {
              providerRef = "Provider/provider-declared";
              componentId = "declared-service";
              method = "export";
              presentation = "filesystem-presentation";
            }
          ];
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "method export is answered by declared-service with presentation filesystem-presentation, which Provider/provider-declared does not declare"
        ];
      };
    };

    "graph-policy/a-registration-naming-an-undeclared-provider-is-refused" = {
      expr = refusal {
        projection = canonical // {
          registrations = [
            {
              artifactId = "provider-retired";
              providerRef = "Provider/provider-retired";
              services = [ ];
            }
          ];
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "registration provider-retired names Provider/provider-retired, which no projection row declares"
        ];
      };
    };

    "graph-policy/a-service-resolving-to-an-undeclared-provider-is-refused" = {
      expr = refusal {
        projection = canonical // {
          serviceCatalog."orphan.d2bus.org/export" = "Provider/provider-orphan";
        };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "service orphan.d2bus.org/export resolves to Provider/provider-orphan, which no projection row declares"
        ];
      };
    };

    "graph-policy/a-projection-under-the-old-contract-version-is-refused" = {
      expr = refusal {
        projection = canonical // { contractVersion = "d2b.zone.v2"; };
      };
      expected = {
        refused = { success = false; value = false; };
        refusals = [
          "projection declares contract version d2b.zone.v2; the canonical graph policy compiles only d2b.zone.v3"
        ];
      };
    };

    # The projection is a pure function of its input, so the same
    # declaration compiles the same policy every time.
    "graph-policy/repeated-projection-is-byte-stable" = {
      expr = builtins.toJSON (policy { projection = canonical; })
        == builtins.toJSON (policy { projection = canonical; });
      expected = true;
    };
  };
}
