{ identity, lib }:

let
  sortNames = lib.sort lib.lessThan;
  sortedNames = attrs: sortNames (lib.attrNames attrs);
  sortUnique = values: sortNames (lib.unique values);

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

  attrPathOr = path: fallback: attrs:
    lib.attrByPath path fallback attrs;

  workloadCapabilities = workload:
    sortUnique (
      (attrPathOr [ "capabilityRefs" ] [ ] workload)
      ++ (attrPathOr [ "launcher" "capabilities" ] [ ] workload)
      ++ lib.optionals (attrPathOr [ "shell" "enable" ] false workload)
        [ "persistent-shell" "pty" ]
    );

  providerRefs = workload:
    let
      refs = attrPathOr [ "providerRefs" ] { } workload;
      runtimeRef = attrPathOr [ "runtime" "provider" ]
        (workload.provider or null) workload;
    in
    refs // lib.optionalAttrs (runtimeRef != null) { runtime = runtimeRef; };
in
{ realms, realmIndex }:
let
  rows = lib.concatMap
    (realmRow:
      let
        realm = realms.${realmRow.realmName};
        workloads = realm.workloads or { };
      in
      map
        (workloadName:
          let
            workload = workloads.${workloadName};
            canonicalName = identity.validateCanonicalName "workload name"
              (workload.id or workloadName);
            workloadId = identity.deriveWorkloadId realmRow.realmId canonicalName;
            canonicalTarget = "${canonicalName}.${realmRow.realmPath}.d2b";
            launcher = workload.launcher or { };
          in
          {
            inherit canonicalTarget workloadId workloadName;
            realmId = realmRow.realmId;
            realmName = realmRow.realmName;
            realmPath = realmRow.realmPath;
            enabled = realmRow.enabled && (workload.enable or true);
            configuredName = canonicalName;
            providerRefs = providerRefs workload;
            capabilityRefs = workloadCapabilities workload;
            metadata = {
              label =
                if (launcher.label or null) != null
                then launcher.label
                else workload.name or canonicalName;
              icon = launcher.icon or { };
            };
            launcher = {
              enabled = launcher.enable or false;
              defaultItem = launcher.defaultItem or null;
              items = launcher.items or { };
            };
            spec = builtins.removeAttrs workload
              [ "enable" "id" "name" "launcher" ];
          })
        (sortedNames workloads))
    realmIndex.list;

  enabled = lib.filter (row: row.enabled) rows;
  by = field: values:
    lib.listToAttrs (map (row: {
      name = row.${field};
      value = row;
    }) values);
  byRealm = realmRows:
    lib.listToAttrs (map
      (realm: {
        name = realm.realmId;
        value = lib.filter (row: row.realmId == realm.realmId) realmRows;
      })
      realmIndex.list);

  validated = requireUnique "workload id" (map (row: row.workloadId) rows);
in
assert validated;
{
  list = rows;
  enabledList = enabled;
  byId = by "workloadId" rows;
  byCanonicalTarget = by "canonicalTarget" rows;
  byRealmId = byRealm rows;
  enabledByRealmId = byRealm enabled;
  ids = map (row: row.workloadId) rows;
  canonicalTargets = map (row: row.canonicalTarget) rows;
}
