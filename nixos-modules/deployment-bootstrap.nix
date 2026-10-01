# The verified deployment graph (U31, KTD7).
#
# One constructor, two publications. The Host publishes the graph that names
# the system Zone before it starts any provider, and every Zone's Guests
# publish their own copy naming that Zone before they serve their target.
# The documents differ in exactly one field - the Zone they name - and share
# the schema tag, the canonical encoding, and the framed self-hash domain, so
# the daemon's verification and a Guest's verification are the same check
# over the same bytes rather than two producers that can drift.
#
# This is deliberately a plain function of the repository's own generated
# declarations rather than a module option: the Host publication and the
# per-Zone Guest publication read the same declarations here, so there is no
# second inventory of implementations or authority rows to maintain.
{ lib }:

let
  resourcesBundle = import ./resources-bundle.nix { inherit lib; };

  # The implementation identities this deployment publishes, read from the
  # same per-crate `registrations.json` declarations the daemon's generated
  # provider registration table is emitted from. Reading the declarations
  # themselves rather than any projection-owner or catalog list is what
  # keeps the two sides from drifting: there is no second inventory to
  # maintain, and the two framework execution providers the foundation seed
  # binds are added from their own crate declarations.
  deploymentProviderCrates = builtins.filter
    (name: lib.hasPrefix "d2b-provider-" name && builtins.pathExists (
      ./../packages/${name}/registrations.json
    ))
    (builtins.attrNames (builtins.readDir ../packages));
  deploymentRegistrations = builtins.map
    (name:
      (builtins.fromJSON (builtins.readFile (
        ./../packages/${name}/registrations.json
      ))).provider)
    deploymentProviderCrates;
  # The identities are published in sorted order so the document's bytes do
  # not depend on readdir order. `builtins.sort` takes an ordering predicate,
  # and the Nix this repository evaluates with has no `compareStrings` builtin
  # to hand it.
  deploymentImplementations = builtins.sort builtins.lessThan
    deploymentRegistrations;

  # The foundation's own process provider, taken from the generated Provider
  # catalog's `fixedBootstrapProviders` rather than written here. That list is
  # the declaration of which providers bootstrap this deployment, so the
  # shared module never names a provider identity of its own: it reads the
  # generated one and constructs the resource reference the foundation
  # self-binding is committed under. `noBinaryBootstrapProvider` is the
  # catalog's own non-binary member, so the remaining entry is the process
  # provider whose self-binding authorizes materialization.
  providerCatalogShape = import ./generated/provider-catalog-shape.nix;
  foundationProcessProvider = lib.head (lib.filter
    (name: name != providerCatalogShape.artifactLayout.noBinaryBootstrapProvider)
    providerCatalogShape.artifactLayout.fixedBootstrapProviders);

  # The authority rows are the fixed foundation vocabulary: the publisher
  # Role and the Process provider's self-binding. They are ordinary graph
  # rows with canonical admitted bytes, evaluated by the one admission
  # evaluator, exactly as every other Role and RoleBinding is.
  publisherRole = {
    rules = [
      {
        resourceTypes = [ "Operation" ];
        verbs = [ "create" ];
        subresources = [ ];
        resourceNames = [ ];
        zones = [ ];
        executionRefs = [ ];
        sessionVerbs = [ ];
      }
    ];
    operationRefs = [ ];
  };
  foundationAuthorityRows = [
    {
      reference = "Role/operation-publisher";
      admitted = publisherRole;
    }
    {
      reference = "RoleBinding/${foundationProcessProvider}-self-operation-publisher";
      admitted = {
        roleRef = "Role/operation-publisher";
        subjects = [ "Provider/${foundationProcessProvider}" ];
      };
    }
  ];
in
rec {
  # The document schema tag this release publishes. The daemon and every
  # Guest verify against the same constant, so a document of another contract
  # version is refused rather than partially understood.
  schemaVersion = "d2b-deployment-bootstrap/1";

  # The domain tag framing the document's own self-hash. It is the profile
  # the Rust decoder re-verifies, so the producer and the verifier cannot
  # disagree about which digest covers the preimage.
  digestDomain = "d2b:v3:deployment-bootstrap";

  # The deployment-root-relative file name both halves read.
  fileName = "deployment-bootstrap.json";

  # The Zone the Host publication names. It is the Host's own deployment
  # graph, not a Zone's: a Guest never reads this one.
  systemZone = "system";

  # The canonical bytes the self-hash covers: this document for one Zone
  # with no `graphDigest` field yet.
  preimageFor = zoneName: {
    inherit schemaVersion;
    zone = zoneName;
    storeIncarnation = "foundation-1";
    stateVolume = "Volume/d2b-state";
    implementations = deploymentImplementations;
    # Row selection is by reference prefix, so the published vocabulary is
    # exactly the Role and RoleBinding rows the foundation declares and
    # nothing else. (`builtins.match` is not a prefix test on the Nix this
    # repository evaluates with, so the prefix is spelled out here.)
    roles = builtins.filter (row: lib.hasPrefix "Role/" row.reference)
      foundationAuthorityRows;
    roleBindings =
      builtins.filter (row: lib.hasPrefix "RoleBinding/" row.reference)
        foundationAuthorityRows;
  };

  # One publication's rendered bytes: the canonical preimage, the framed
  # digest over it, and the document that carries that digest. The digest is
  # computed over the preimage before the field exists, which is exactly the
  # byte string the reader reconstructs when it verifies.
  documentFor = zoneName:
    let
      preimage = preimageFor zoneName;
      preimageJson =
        builtins.toJSON (resourcesBundle.canonical preimage);
      graphDigest =
        "sha256:${resourcesBundle.framedDigest digestDomain preimageJson}";
    in {
      inherit preimageJson graphDigest;
      path = fileName;
      documentJson = builtins.toJSON (resourcesBundle.canonical
        (preimage // { inherit graphDigest; }));
    };
}