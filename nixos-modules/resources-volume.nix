# Volume resource validation.
#
# Volume is the single shareable-storage ResourceType. This module adds
# the eval-time half of the Volume contract over the Zone resource
# attrset declared in `options-zones.nix`: every assertion here fails the
# build before anything is rendered, and none of them names a host path,
# a secret, or a resolved source root.
#
# A Volume source is an opaque policy ID. The raw root it stands for
# lives only in the selected Provider's private configuration, so this
# module validates the ID's shape and never resolves it.
{ config, lib, ... }:

let
  cfg = config.d2b;

  maxLayoutEntries = 1024;
  maxViews = 64;
  maxAttachments = 64;
  maxLayoutPathBytes = 255;

  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  modePattern = "^[0-7][0-7][0-7][0-7]$";
  permissionsPattern = "^[rwx]{0,3}$";

  parseRef = ref:
    let parts = lib.splitString "/" ref;
    in {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    };

  resolvesAs = resources: expectedType: ref:
    let parsed = parseRef ref;
    in parsed.type == expectedType
    && builtins.hasAttr parsed.name resources
    && resources.${parsed.name}.type == expectedType;

  isExecutionRef = resources: ref:
    resolvesAs resources "Host" ref || resolvesAs resources "Guest" ref;

  # An anchored layout path is relative to the Volume root. The empty
  # string is the root itself.
  anchoredPath = value:
    builtins.isString value
    && builtins.stringLength value <= maxLayoutPathBytes
    && !(lib.hasPrefix "/" value)
    && !(lib.hasInfix "\\" value)
    && !(builtins.elem ".." (lib.splitString "/" value));

  # A guest-side mount path is absolute and still carries no traversal.
  guestMountPath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && builtins.stringLength value <= maxLayoutPathBytes
    && !(builtins.elem ".." (lib.splitString "/" value));

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  aclAssertions = path: resources: entryIndex: field: grants:
    lib.flatten (lib.imap0
      (grantIndex: grant:
        let where = "${path}.layout.${toString entryIndex}.${field}.${toString grantIndex}";
        in [
          {
            assertion =
              builtins.isAttrs grant
              && builtins.hasAttr "principal" grant
              && builtins.hasAttr "ref" grant.principal
              && resolvesAs resources "User" grant.principal.ref;
            message = "${where}.principal.ref must be a User in the same Zone; a numeric identity is not accepted.";
          }
          {
            assertion =
              builtins.match permissionsPattern (attrOr grant "permissions" "") != null;
            message = "${where}.permissions must be a POSIX rwx string.";
          }
        ])
      grants);

  layoutAssertions = path: resources: layout:
    [
      {
        assertion = lib.length layout <= maxLayoutEntries;
        message = "${path}.layout must contain at most ${toString maxLayoutEntries} entries.";
      }
      {
        assertion =
          lib.length (lib.unique (map (entry: attrOr entry "path" null) layout))
          == lib.length layout;
        message = "${path}.layout entries must declare unique anchored paths.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: entry:
        let
          where = "${path}.layout.${toString index}";
          entryType = attrOr entry "type" null;
          target = attrOr entry "target" null;
        in
        [
          {
            assertion = anchoredPath (attrOr entry "path" null);
            message = "${where}.path must be anchored inside the Volume: relative, no '..' component, no backslash.";
          }
          {
            assertion = builtins.elem entryType [ "directory" "file" "symlink" "unix-socket" ];
            message = "${where}.type must be directory, file, symlink, or unix-socket.";
          }
          {
            assertion = resolvesAs resources "User" (attrOr entry "ownerRef" "");
            message = "${where}.ownerRef must resolve to a User in the same Zone.";
          }
          {
            assertion = resolvesAs resources "User" (attrOr entry "groupRef" "");
            message = "${where}.groupRef must resolve to a User in the same Zone.";
          }
          {
            assertion = builtins.match modePattern (attrOr entry "mode" "") != null;
            message = "${where}.mode must be a four-digit octal string.";
          }
          {
            assertion = entryType != "symlink" -> target == null;
            message = "${where}.target is accepted only for a symlink entry.";
          }
          {
            assertion = entryType == "symlink" -> (target != null && anchoredPath target);
            message = "${where}.target is required for a symlink and must resolve inside the Volume root.";
          }
          {
            assertion = entryType == "symlink" -> attrOr entry "noFollow" true == false;
            message = "${where}.noFollow must be false for a symlink entry and true otherwise.";
          }
          {
            assertion = entryType != "symlink" -> attrOr entry "noFollow" true == true;
            message = "${where}.noFollow must be true for every entry that is not a symlink.";
          }
          {
            assertion = builtins.elem (attrOr entry "foreignChildPolicy" "preserve") [ "preserve" "fail" ];
            message = "${where}.foreignChildPolicy must be preserve or fail.";
          }
        ]
        ++ aclAssertions path resources index "accessAcl" (attrOr entry "accessAcl" [ ])
        ++ aclAssertions path resources index "defaultAcl" (attrOr entry "defaultAcl" [ ]))
      layout);

  viewAssertions = path: views:
    [
      {
        assertion = views != { };
        message = "${path}.views must declare at least one named view.";
      }
      {
        assertion = lib.length (builtins.attrNames views) <= maxViews;
        message = "${path}.views must contain at most ${toString maxViews} views.";
      }
    ]
    ++ lib.flatten (lib.mapAttrsToList
      (name: view:
        let where = "${path}.views.${name}";
        in [
          {
            assertion = builtins.match tokenPattern name != null;
            message = "${where}: view name must match ${tokenPattern}.";
          }
          {
            assertion = anchoredPath (attrOr view "path" null);
            message = "${where}.path must be anchored inside the Volume.";
          }
          {
            assertion =
              let rights = attrOr view "rights" [ ];
              in rights != [ ]
              && lib.length (lib.unique rights) == lib.length rights
              && lib.all
                (right: builtins.elem right [ "read" "write" "create" "delete" "traverse" "execute" ])
                rights;
            message = "${where}.rights must be a non-empty set of unique known rights.";
          }
        ])
      views);

  attachmentAssertions = path: resources: views: attachments:
    let
      writers = lib.filter (a: attrOr a "access" "read-only" == "read-write") attachments;
    in
    [
      {
        assertion = lib.length attachments <= maxAttachments;
        message = "${path}.attachments must contain at most ${toString maxAttachments} attachments.";
      }
      {
        assertion = lib.length writers <= 1;
        message = "${path}.attachments may declare at most one read-write attachment.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: attachment:
        let
          where = "${path}.attachments.${toString index}";
          view = attrOr attachment "view" null;
          access = attrOr attachment "access" "read-only";
          rights = attrOr (attrOr views view { }) "rights" [ ];
        in
        [
          {
            assertion = isExecutionRef resources (attrOr attachment "executionRef" "");
            message = "${where}.executionRef must resolve to a Host or Guest in the same Zone.";
          }
          {
            assertion = builtins.elem (attrOr attachment "transport" null) [ "virtiofs" "virtio-blk" ];
            message = "${where}.transport must be virtiofs or virtio-blk.";
          }
          {
            assertion = view != null && builtins.hasAttr view views;
            message = "${where}.view must name a view the Volume declares.";
          }
          {
            assertion = builtins.elem access [ "read-only" "read-write" "shared-write" ];
            message = "${where}.access must be read-only, read-write, or shared-write.";
          }
          {
            assertion = access == "read-only" || builtins.elem "write" rights;
            message = "${where}.access requires the selected view to grant the write right.";
          }
          {
            assertion = guestMountPath (attrOr attachment "mountPath" null);
            message = "${where}.mountPath must be an absolute guest-side path with no '..' component.";
          }
        ])
      attachments);

  sourceAssertions = path: resources: source: quota:
    let
      settings = attrOr source "settings" { };
      kind = attrOr settings "kind" null;
      policyId = attrOr settings "sourcePolicyId" null;
      hostBacked = builtins.elem kind [ "local-path" "block-image" ];
      maxBytes = attrOr quota "maxBytes" null;
      maxInodes = attrOr quota "maxInodes" null;
    in
    [
      {
        assertion = isExecutionRef resources (attrOr source "executionRef" "");
        message = "${path}.source.executionRef must resolve to a Host or Guest in the same Zone.";
      }
      {
        assertion = builtins.elem kind [ "local-path" "block-image" "tmpfs" ];
        message = "${path}.source.settings.kind must be local-path, block-image, or tmpfs.";
      }
      {
        assertion = !(builtins.hasAttr "path" settings) && !(builtins.hasAttr "hostPath" settings);
        message = "${path}.source.settings must not carry a host path; a Volume source is an opaque policy ID.";
      }
      {
        assertion = hostBacked -> (policyId != null && builtins.match tokenPattern policyId != null);
        message = "${path}.source.settings.sourcePolicyId is required for a host-backed source and must match ${tokenPattern}.";
      }
      {
        assertion = !hostBacked -> policyId == null;
        message = "${path}.source.settings.sourcePolicyId is accepted only for a host-backed source.";
      }
      {
        assertion = kind != "block-image" || maxBytes != null;
        message = "${path}.quota.maxBytes is required for a block-image source.";
      }
      {
        assertion = kind != "tmpfs" || (maxBytes != null && maxInodes != null);
        message = "${path}.quota.maxBytes and ${path}.quota.maxInodes are required for a tmpfs source.";
      }
    ];

  volumeAssertions = zoneName: resourceName: resources: resource:
    let
      path = "d2b.zones.${zoneName}.resources.${resourceName}";
      spec = resource.spec;
      views = attrOr spec "views" { };
    in
    [
      {
        assertion = resolvesAs resources "Provider" (attrOr spec "providerRef" "");
        message = "${path}.spec.providerRef must resolve to a Provider in Zone ${zoneName}.";
      }
      {
        assertion = builtins.elem (attrOr spec "kind" null) [ "durable" "ephemeral" "state" "tmp" "cache" ];
        message = "${path}.spec.kind must be durable, ephemeral, state, tmp, or cache.";
      }
    ]
    ++ sourceAssertions "${path}.spec" resources (attrOr spec "source" { }) (attrOr spec "quota" { })
    ++ layoutAssertions "${path}.spec" resources (attrOr spec "layout" [ ])
    ++ viewAssertions "${path}.spec" views
    ++ attachmentAssertions "${path}.spec" resources views (attrOr spec "attachments" [ ]);

  zoneVolumeAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      lib.flatten (lib.mapAttrsToList
        (resourceName: resource:
          lib.optionals (resource.type == "Volume")
            (volumeAssertions zoneName resourceName zone.resources resource))
        zone.resources))
    cfg.zones);
in
{
  config.assertions = zoneVolumeAssertions;
}
