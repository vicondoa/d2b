{ config, lib, ... }:

let
  cfg = config.d2b;
  stripNulls = lib.filterAttrs (_: value: value != null);
  realmRows = lib.sortOn (row: row.realmPath) cfg._index.realms.enabledList;

  identityEntry =
    realm:
    let
      keys = cfg.realms.${realm.realmName}.keys;
    in
    stripNulls {
      realm = lib.splitString "." realm.realmPath;
      realmIdentityRef = keys.realmIdentityRef;
      realmIdentityFingerprint = keys.realmIdentityFingerprint;
      controllerCredentialRef = keys.controllerKeyRef;
      controllerCredentialFingerprint = keys.controllerCredentialFingerprint;
      trustBundleRef = keys.trustBundleRef;
      enrollmentRef = keys.enrollmentRef;
      rotationPolicyRef = keys.rotationPolicyRef;
    };

  hasIdentityMetadata =
    realm:
    let
      keys = cfg.realms.${realm.realmName}.keys;
    in
    keys.realmIdentityRef != null
    || keys.realmIdentityFingerprint != null
    || keys.controllerKeyRef != null
    || keys.controllerCredentialFingerprint != null
    || keys.trustBundleRef != null
    || keys.enrollmentRef != null
    || keys.rotationPolicyRef != null;

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
