{ config, lib, ... }:

let
  cfg = config.d2b;

  sortNames = names: lib.sort lib.lessThan names;

  stripNulls = lib.filterAttrs (_: value: value != null);
  realmLabels = path: lib.splitString "." path;
  identityEntry = realm:
    stripNulls {
      realm = realmLabels realm.path;
      realmIdentityRef = realm.keys.realmIdentityRef;
      realmIdentityFingerprint = realm.keys.realmIdentityFingerprint;
      controllerCredentialRef = realm.keys.controllerKeyRef;
      controllerCredentialFingerprint = realm.keys.controllerCredentialFingerprint;
      trustBundleRef = realm.keys.trustBundleRef;
      enrollmentRef = realm.keys.enrollmentRef;
      rotationPolicyRef = realm.keys.rotationPolicyRef;
    };

  hasIdentityMetadata = realm:
    realm.keys.realmIdentityRef != null
    || realm.keys.realmIdentityFingerprint != null
    || realm.keys.controllerKeyRef != null
    || realm.keys.controllerCredentialFingerprint != null
    || realm.keys.trustBundleRef != null
    || realm.keys.enrollmentRef != null
    || realm.keys.rotationPolicyRef != null;

  realmRows =
    map
      (_: cfg._index.realms.enabledByPath.${_})
      (sortNames (lib.attrNames cfg._index.realms.enabledByPath));

  data = {
    schemaVersion = "v2";
    runtimeState = "metadata-only";
    realms = map identityEntry (lib.filter hasIdentityMetadata realmRows);
    invariants = {
      metadataOnly = true;
      noSecretMaterial = true;
      preservesRuntimeBehavior = true;
    };
  };
in
{
  config.d2b._bundle.realmIdentityJson = {
    inherit data;
    installFileName = "realm-identity.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
