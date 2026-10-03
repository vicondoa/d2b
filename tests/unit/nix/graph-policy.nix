# The isolated new-graph policy projection.
#
# This module is the Nix half of the new graph's build/test closure. It
# turns one declaration projection - the document the Rust projection emits
# from a provider declaration and nothing else - into the canonical graph
# policy a configuration consumer reads: which Provider each component
# belongs to, what presentation it declared, which setup restrictions that
# presentation requires, which declared method a call addresses, and which
# consumer request a binding slot compiles to.
#
# It is a test artifact, not a production module. Exactly one surface
# imports it, it declares no NixOS option, and no public option module
# imports it, so staging the new graph's Nix half cannot change what
# `nixosModules.default` evaluates to.
#
# # What it refuses
#
# A projection is data, and data that still speaks the retired vocabulary is
# not a projection. The retired knobs are named, closed, and refused rather
# than ignored: a caller that passes one gets a diagnostic naming the knob
# instead of a silently narrower policy. The same rule applies to every
# projected row: each has a closed key set, so a row that smuggles a
# privilege table, a role-scope map or a wire-variant list back in is refused
# before any policy is built rather than being read as if it were absent.
{ lib }:

{ projection
, legacy ? { }
}:

let
  # The knobs the new graph retired. Each was a second source: the
  # handwritten privilege copy, the role and family scope tables, the broker
  # wire-variant list, and the principal allocation. None is an input here.
  retiredKnobs = [
    "brokerOperations"
    "familyScopes"
    "privileges"
    "principalAllocation"
    "roleScopes"
    "wireVariants"
  ];

  refusedKnobs = builtins.filter (knob: builtins.hasAttr knob legacy) retiredKnobs;

  # The contract version the new graph compiles. A projection under any
  # other version is refused rather than downgraded: the release is a clean
  # break, so there is no compatibility read.
  contractVersion = "d2b.zone.v3";

  # The closed key set of each projected row. The projection states an
  # identity, a digest, or a declared relationship, and nothing else.
  keySets = {
    projection = [
      "consumerRequests"
      "contractVersion"
      "operations"
      "providers"
      "registrations"
      "serviceCatalog"
    ];
    provider = [
      "components"
      "configDigest"
      "declarationDigest"
      "executableSetDigest"
      "providerRef"
    ];
    component = [
      "componentId"
      "presentation"
      "setupRestrictions"
    ];
    registration = [
      "artifactId"
      "providerRef"
      "services"
    ];
    operation = [
      "componentId"
      "method"
      "presentation"
      "providerRef"
    ];
    consumerRequest = [
      "consumerRef"
      "fingerprint"
      "kind"
      "slot"
      "zone"
    ];
  };

  contractVersion_ = projection.contractVersion or null;
  providers = projection.providers or { };
  serviceCatalog = projection.serviceCatalog or { };
  registrations = projection.registrations or [ ];
  operations = projection.operations or [ ];
  consumerRequests = projection.consumerRequests or [ ];

  providerNames = builtins.attrNames providers;

  # The keys one value carries that its closed key set does not name.
  extraKeys = keys: value:
    builtins.filter (key: !(builtins.elem key keys)) (builtins.attrNames value);

  # The rows whose key set is wider than the projection states.
  wideRows =
    (if extraKeys keySets.projection projection != [ ] then
      [ "the projection carries ${builtins.concatStringsSep ", " (extraKeys keySets.projection projection)}" ]
    else [ ])
    ++ builtins.concatMap
      (name:
        let row = providers.${name};
        in
        (if extraKeys keySets.provider row != [ ] then
          [
            "provider ${name} carries ${builtins.concatStringsSep ", " (extraKeys keySets.provider row)}"
          ]
        else [ ])
        ++ map
          (component: "component ${name}/${component.componentId} carries ${builtins.concatStringsSep ", " (extraKeys keySets.component component)}")
          (builtins.filter (component: extraKeys keySets.component component != [ ]) row.components))
      providerNames
    ++ map
      (row: "registration ${row.artifactId} carries ${builtins.concatStringsSep ", " (extraKeys keySets.registration row)}")
      (builtins.filter (row: extraKeys keySets.registration row != [ ]) registrations)
    ++ map
      (row: "operation ${row.method} carries ${builtins.concatStringsSep ", " (extraKeys keySets.operation row)}")
      (builtins.filter (row: extraKeys keySets.operation row != [ ]) operations)
    ++ map
      (row: "consumer request ${row.slot} carries ${builtins.concatStringsSep ", " (extraKeys keySets.consumerRequest row)}")
      (builtins.filter (row: extraKeys keySets.consumerRequest row != [ ]) consumerRequests);

  # Every component the projection declares, flattened to the triple an
  # operation row is checked against.
  declaredComponents = lib.concatLists (map
    (name: map
      (component: {
        providerRef = providers.${name}.providerRef;
        inherit (component) componentId presentation;
      })
      providers.${name}.components)
    providerNames);

  # A method is reachable only when the Provider it names declares the
  # component that answers it, with the presentation that component already
  # carries. The canonical policy states the declared presentation once, so a
  # row that names an undeclared component, or restates a different one, is
  # refused rather than preferred.
  unroutableOperations = builtins.filter
    (row: !(lib.any
      (component:
        component.providerRef == row.providerRef
        && component.componentId == row.componentId
        && component.presentation == row.presentation)
      declaredComponents))
    operations;

  unroutable = map
    (row:
      "method ${row.method} is answered by ${row.componentId} with presentation ${row.presentation}, which ${row.providerRef} does not declare")
    unroutableOperations;

  # The registrations and catalog entries naming a Provider the projection
  # does not carry. A registration with no provider row has no declaration
  # behind it, and a catalog entry pointing at one would route a service to
  # nothing.
  knownProviderRefs = map (name: providers.${name}.providerRef) providerNames;
  orphanRegistrations = map
    (row: "registration ${row.artifactId} names ${row.providerRef}, which no projection row declares")
    (builtins.filter (row: !(builtins.elem row.providerRef knownProviderRefs)) registrations);
  orphanServices = map
    (service: "service ${service} resolves to ${serviceCatalog.${service}}, which no projection row declares")
    (builtins.filter
      (service: !(builtins.elem serviceCatalog.${service} knownProviderRefs))
      (builtins.attrNames serviceCatalog));

  failures = lib.concatLists [
    (if refusedKnobs != [ ] then
      [
        (builtins.concatStringsSep " " [
          "refused retired knob(s) ${builtins.concatStringsSep ", " refusedKnobs};"
          "the canonical graph policy is derived from the provider declaration"
          "and admits no second source"
        ])
      ]
    else [ ])
    (if contractVersion_ != contractVersion then
      [
        (builtins.concatStringsSep " " [
          "projection declares contract version ${toString contractVersion_};"
          "the canonical graph policy compiles only ${contractVersion}"
        ])
      ]
    else [ ])
    (if wideRows != [ ] then wideRows else [ ])
    (if unroutable != [ ] then unroutable else [ ])
    (if orphanRegistrations != [ ] then orphanRegistrations else [ ])
    (if orphanServices != [ ] then orphanServices else [ ])
  ];
in
{
  # The refusals are a value, not only a thrown message. A caller that can
  # read them asserts WHICH refusal a projection earned; a throw alone only
  # says that something refused, which every unrelated defect also satisfies.
  refusals = failures;

  policy =
    if failures != [ ] then
      throw ''
        graph-policy: the projection is not a canonical new-graph policy:
        ${builtins.concatStringsSep "\n    - " failures}
      ''
    else
      {
        inherit contractVersion;
        providerRefs = builtins.sort lib.lessThan knownProviderRefs;
        components = lib.concatLists (map
          (name: map
            (component: {
              inherit (component) componentId presentation setupRestrictions;
              providerRef = providers.${name}.providerRef;
              declarationDigest = providers.${name}.declarationDigest;
            })
            providers.${name}.components)
          providerNames);
        methods = operations;
        consumerRequests = consumerRequests;
      };
}
