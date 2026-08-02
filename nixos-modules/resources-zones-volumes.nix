# Volume resource compiler.
#
# Volume is the only v3 storage resource. Host paths and numeric identities
# remain private Provider inputs; the public resource carries policy IDs and
# typed User/Host/Guest references only.
{ config, lib, ... }:

let
  cfg = config.d2b;
  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  modePattern = "^[0-7]{4}$";
  rights = [ "read" "write" "create" "delete" "traverse" "execute" ];
  volumeKinds = [ "durable" "ephemeral" "state" "tmp" "cache" ];
  sourceKinds = [ "local-path" "block-image" "tmpfs" "nix-closure" ];

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: types: value:
    let parsed = parseRef value;
    in parsed != null
      && builtins.elem parsed.type types
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  anchoredPath = value:
    builtins.isString value
    && builtins.stringLength value <= 255
    && !(lib.hasPrefix "/" value)
    && !(lib.hasPrefix "\\" value)
    && !(lib.hasInfix "\\" value)
    && !(lib.hasInfix ":" value)
    && !(builtins.elem ".." (lib.splitString "/" value))
    && (value == "" || lib.all
      (component: component != "" && component != ".")
      (lib.splitString "/" value));

  guestPath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && builtins.stringLength value <= 255
    && !(builtins.elem ".." (lib.splitString "/" value));

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (lib.attrNames value);

  artifactFor = artifactId:
    if builtins.isString artifactId && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  rows = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource zone;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          spec = resource.spec or { };
        })
        (lib.filterAttrs (_: resource: resource.type == "Volume") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  aclAssertions = row: index: field: grants:
    lib.flatten (lib.imap0
      (grantIndex: grant:
        let path = "${row.path}.spec.layout.${toString index}.${field}.${toString grantIndex}";
        in [
          {
            assertion = builtins.isAttrs grant
              && resolvesAs row.zone.resources [ "User" ] ((grant.principal or { }).ref or null);
            message = "${path}.principal.ref must resolve to a User in the same Zone.";
          }
          {
            assertion = builtins.match "^[rwx-]{1,3}$" (grant.permissions or "") != null;
            message = "${path}.permissions must be a POSIX rwx string.";
          }
        ])
      grants);

  layoutAssertions = row:
    let
      layout = row.spec.layout or [ ];
      checks = lib.flatten (lib.imap0
        (index: entry:
          let
            path = "${row.path}.spec.layout.${toString index}";
            type = entry.type or null;
            target = entry.target or null;
          in [
            {
              assertion = exactKeys [
                "path" "type" "target" "ownerRef" "groupRef" "mode"
                "noFollow" "createPolicy" "repairPolicy" "cleanupPolicy"
                "foreignChildPolicy" "accessAcl" "defaultAcl"
              ] entry;
              message = "${path} contains an unsupported layout field.";
            }
            {
              assertion = anchoredPath (entry.path or null);
              message = "${path}.path must be relative and free of traversal.";
            }
            {
              assertion = builtins.elem type [ "directory" "file" "symlink" "unix-socket" ];
              message = "${path}.type is not a supported layout type.";
            }
            {
              assertion = resolvesAs row.zone.resources [ "User" ] (entry.ownerRef or null)
                && resolvesAs row.zone.resources [ "User" ] (entry.groupRef or null);
              message = "${path}.ownerRef and groupRef must resolve to Users.";
            }
            {
              assertion = builtins.match modePattern (entry.mode or "") != null;
              message = "${path}.mode must be a four-digit octal string.";
            }
            {
              assertion = type == "symlink"
                || (entry.noFollow or true) == true;
              message = "${path}.noFollow must be true for non-symlink entries.";
            }
            {
              assertion = type != "symlink"
                || ((entry.noFollow or false) == false && anchoredPath target);
              message = "${path}.symlink target must be relative and no-follow must be false.";
            }
            {
              assertion = builtins.elem (entry.createPolicy or "create-if-missing")
                [ "create-if-missing" "create-if-never-provisioned" "must-exist" ];
              message = "${path}.createPolicy is invalid.";
            }
            {
              assertion = builtins.elem (entry.repairPolicy or "exact-owner")
                [ "exact-owner" "preserve" "fail" ];
              message = "${path}.repairPolicy is invalid.";
            }
            {
              assertion = builtins.elem (entry.cleanupPolicy or "owner-controlled")
                [ "owner-controlled" "preserve" "remove" ];
              message = "${path}.cleanupPolicy is invalid.";
            }
            {
              assertion = builtins.elem (entry.foreignChildPolicy or "preserve")
                [ "preserve" "fail" ];
              message = "${path}.foreignChildPolicy is invalid.";
            }
          ]
          ++ aclAssertions row index "accessAcl" (entry.accessAcl or [ ])
          ++ aclAssertions row index "defaultAcl" (entry.defaultAcl or [ ]))
        layout);
    in [
      {
        assertion = builtins.isList layout && lib.length layout <= 1024;
        message = "${row.path}.spec.layout must contain at most 1024 entries.";
      }
      {
        assertion = lib.length (lib.unique (map (entry: entry.path or null) layout))
          == lib.length layout;
        message = "${row.path}.spec.layout paths must be unique.";
      }
    ] ++ checks;

  viewAssertions = row:
    let views = row.spec.views or { };
    in [
      {
        assertion = builtins.isAttrs views && views != { }
          && lib.length (lib.attrNames views) <= 64;
        message = "${row.path}.spec.views must contain between one and 64 views.";
      }
    ] ++ lib.flatten (lib.mapAttrsToList
      (name: view:
        let path = "${row.path}.spec.views.${name}";
        in [
          {
            assertion = builtins.match tokenPattern name != null;
            message = "${path}: view name is invalid.";
          }
          {
            assertion = anchoredPath (view.path or null);
            message = "${path}.path must be relative and free of traversal.";
          }
          {
            assertion = builtins.isList (view.rights or [ ])
              && (view.rights or [ ]) != [ ]
              && lib.length (lib.unique (view.rights or [ ])) == lib.length (view.rights or [ ])
              && lib.all (right: builtins.elem right rights) (view.rights or [ ]);
            message = "${path}.rights must be a non-empty unique set of known rights.";
          }
        ])
      views);

  sourceAssertions = row:
    let
      source = row.spec.source or { };
      settings = source.settings or { };
      kind = settings.kind or source.kind or null;
      sourcePolicyId = settings.sourcePolicyId or null;
      systemArtifactId = source.systemArtifactId or null;
      artifact = artifactFor systemArtifactId;
    in [
      {
        assertion = resolvesAs row.zone.resources [ "Host" "Guest" ] (source.executionRef or null);
        message = "${row.path}.spec.source.executionRef must resolve to a Host or Guest.";
      }
      {
        assertion = builtins.elem kind sourceKinds;
        message = "${row.path}.spec.source kind is invalid.";
      }
      {
        assertion = !(builtins.hasAttr "path" settings)
          && !(builtins.hasAttr "hostPath" settings);
        message = "${row.path}.spec.source.settings must not contain a raw host path.";
      }
      {
        assertion = !(builtins.elem kind [ "local-path" "block-image" ])
          || (builtins.isString sourcePolicyId
            && builtins.match tokenPattern sourcePolicyId != null);
        message = "${row.path}.spec.source.settings.sourcePolicyId is required for host-backed sources.";
      }
      {
        assertion = kind != "nix-closure"
          || (builtins.isString systemArtifactId
            && artifact != null
            && (artifact.type or null) == "nixos-system");
        message = "${row.path}.spec.source.systemArtifactId must name a nixos-system artifact.";
      }
      {
        assertion = kind == "nix-closure"
          || systemArtifactId == null;
        message = "${row.path}.spec.source.systemArtifactId is valid only for nix-closure sources.";
      }
    ];

  attachmentAssertions = row:
    let
      attachments = row.spec.attachments or [ ];
      check = index: attachment:
        let
          path = "${row.path}.spec.attachments.${toString index}";
          view = attachment.view or null;
          views = row.spec.views or { };
        in [
          {
            assertion = exactKeys [ "executionRef" "transport" "mountPath" "view" "access" ] attachment;
            message = "${path} contains an unsupported attachment field.";
          }
          {
            assertion = resolvesAs row.zone.resources [ "Host" "Guest" ] (attachment.executionRef or null);
            message = "${path}.executionRef must resolve to a Host or Guest.";
          }
          {
            assertion = builtins.elem (attachment.transport or null) [ "virtiofs" "virtio-blk" ];
            message = "${path}.transport is invalid.";
          }
          {
            assertion = (row.spec.source.kind or (row.spec.source.settings.kind or null)) != "block-image"
              || attachment.transport == "virtio-blk";
            message = "${path}: block-image attachments must use virtio-blk.";
          }
          {
            assertion = builtins.hasAttr view views;
            message = "${path}.view must name a declared Volume view.";
          }
          {
            assertion = builtins.elem (attachment.access or "read-only")
              [ "read-only" "read-write" "shared-write" ];
            message = "${path}.access is invalid.";
          }
          {
            assertion = (attachment.access or "read-only") == "read-only"
              || builtins.elem "write" (views.${view}.rights or [ ]);
            message = "${path}.access requires write in the selected view.";
          }
          {
            assertion = guestPath (attachment.mountPath or null);
            message = "${path}.mountPath must be an absolute path without traversal.";
          }
        ];
    in [
      {
        assertion = builtins.isList attachments && lib.length attachments <= 64;
        message = "${row.path}.spec.attachments must contain at most 64 entries.";
      }
      {
        assertion = lib.length (lib.filter
          (attachment: (attachment.access or "read-only") == "read-write")
          attachments) <= 1;
        message = "${row.path}.spec.attachments may contain at most one read-write attachment.";
      }
    ] ++ lib.concatLists (lib.imap0 check attachments);

  volumeAssertions = row:
    let
      spec = row.spec;
      providerRef = spec.providerRef or null;
      provider =
        let parsed = parseRef providerRef;
        in if parsed != null && builtins.hasAttr parsed.name row.zone.resources
          then row.zone.resources.${parsed.name}
          else null;
      artifactId = if provider == null then null else provider.spec.artifactId or null;
      artifact = artifactFor artifactId;
    in [
      {
        assertion = resolvesAs row.zone.resources [ "Provider" ] providerRef;
        message = "${row.path}.spec.providerRef must resolve to a Provider.";
      }
      {
        assertion = artifact != null && (artifact.type or null) == "provider";
        message = "${row.path}.spec.providerRef must select a provider artifact.";
      }
      {
        assertion = builtins.elem (spec.kind or null) volumeKinds;
        message = "${row.path}.spec.kind must be durable, ephemeral, state, tmp, or cache.";
      }
      {
        assertion = exactKeys [
          "providerRef" "updatePolicy" "kind" "source" "layout" "views" "attachments"
          "quota" "identityMarker" "stateSchema" "persistenceClass" "sensitivityClass"
        ] spec;
        message = "${row.path}.spec contains an unsupported Volume field.";
      }
    ]
    ++ sourceAssertions row
    ++ layoutAssertions row
    ++ viewAssertions row
    ++ attachmentAssertions row;

  canonical = row:
    let
      spec = row.spec;
      source = spec.source or { };
      settings = source.settings or { };
      kind = settings.kind or source.kind or null;
      canonicalSource =
        if kind == "nix-closure" then {
          executionRef = source.executionRef or null;
          kind = "nix-closure";
          systemArtifactId = source.systemArtifactId;
        } else {
          executionRef = source.executionRef or null;
          settings = {
            kind = kind;
            sourcePolicyId = settings.sourcePolicyId or null;
          };
        };
    in (builtins.removeAttrs spec [ "source" ]) // { source = canonicalSource; };

  canonicalResource = row: {
    apiVersion = "resources.d2bus.org/v3";
    type = "Volume";
    metadata = {
      name = row.resourceName;
      zone = row.zoneName;
    }
    // lib.optionalAttrs ((row.resource.metadata.ownerRef or null) != null) {
      ownerRef = row.resource.metadata.ownerRef;
    };
    spec = canonical row;
  };

  compiled = lib.foldl'
    (result: row:
      result // {
        ${row.zoneName} = (result.${row.zoneName} or { }) // {
          ${row.resourceName} = canonicalResource row;
        };
      })
    { }
    rows;
in
{
  config = {
    assertions = lib.concatMap volumeAssertions rows;
    d2b._resourceCompiler.volumes = {
      byZone = compiled;
      rows = rows;
    };
  };
}
