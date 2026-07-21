{ identity, lib }:

let
  sortNames = lib.sort lib.lessThan;
  sortedNames = attrs: sortNames (lib.attrNames attrs);

  canonicalRealmPath = path:
    if path == "local-root" || lib.hasSuffix ".local-root" path
    then identity.validateRealmPath path
    else identity.validateRealmPath "${path}.local-root";

  duplicateValues = values:
    lib.unique (lib.filter
      (value: lib.length (lib.filter (candidate: candidate == value) values) > 1)
      values);

  requireUnique = label: values:
    let duplicates = duplicateValues values;
    in
    if duplicates == [ ]
    then true
    else throw "normalized index: duplicate ${label}: ${lib.concatStringsSep ", " duplicates}";
in
realms:
let
  rows = map
    (realmName:
      let
        realm = realms.${realmName};
        configuredPath = realm.path or realmName;
        realmPath = canonicalRealmPath configuredPath;
        realmId = identity.deriveRealmId realmPath;
        parentPath =
          if (realm.parent or null) == null
          then
            if realmPath == "local-root"
            then null
            else
              lib.concatStringsSep "."
                (builtins.tail (lib.splitString "." realmPath))
          else canonicalRealmPath realm.parent;
      in
      {
        inherit realmId realmName realmPath parentPath;
        parentRealmId =
          if parentPath == null then null else identity.deriveRealmId parentPath;
        enabled = realm.enable or true;
        placement = realm.placement or "host-local";
        metadata = {
          configuredId = realm.id or realmName;
          configuredPath = configuredPath;
          name = realm.name or realmName;
        };
        canonicalTargetSuffix = "${realmPath}.d2b";
      })
    (sortedNames realms);

  enabled = lib.filter (row: row.enabled) rows;
  by = field: values:
    lib.listToAttrs (map (row: {
      name = row.${field};
      value = row;
    }) values);

  validated =
    requireUnique "realm path" (map (row: row.realmPath) rows)
    && requireUnique "realm id" (map (row: row.realmId) rows);
in
assert validated;
{
  list = rows;
  enabledList = enabled;
  byId = by "realmId" rows;
  byName = by "realmName" rows;
  byPath = by "realmPath" rows;
  enabledById = by "realmId" enabled;
  enabledByName = by "realmName" enabled;
  enabledByPath = by "realmPath" enabled;
  ids = map (row: row.realmId) rows;
  names = map (row: row.realmName) rows;
  paths = map (row: row.realmPath) rows;
}
