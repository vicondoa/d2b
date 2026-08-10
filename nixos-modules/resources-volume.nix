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
  permissionsPattern = "^[rwx]{1,3}$";
  layoutKeys = [
    "path" "type" "ownerRef" "groupRef" "mode" "target"
    "accessAcl" "defaultAcl" "foreignChildPolicy" "noFollow"
    "recursive" "sensitivity" "createPolicy" "repairPolicy"
    "cleanupPolicy" "adoptionPolicy" "restartPolicy" "leaseClass"
    "invariants"
  ];
  viewKeys = [ "path" "rights" ];
  attachmentKeys = [ "executionRef" "transport" "view" "access" "mountPath" "settings" ];
  attachmentSettingKeys = [
    "posixAcl" "xattr" "cache" "inodeFileHandles" "threadPoolSize" "socketGroup"
  ];
  sourceSettingKeys = [ "kind" "sourcePolicyId" "imageFormat" "preallocate" ];
  quotaKeys = [ "maxBytes" "maxInodes" "enforcement" ];
  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (builtins.attrNames value);

  # Parse a "Type/name" reference, or report that it is not one.
  #
  # The type and length checks are load-bearing. These helpers back assertions,
  # and indexing a split without checking its shape turns a malformed ref into
  # a fatal evaluation abort rather than the assertion message that names the
  # offending option. Returning null lets the caller answer false and let its
  # own assertion report the real problem.
  parseRef = ref:
    let
      parts = if builtins.isString ref then lib.splitString "/" ref else [ ];
    in
    if lib.length parts == 2 then
      {
        type = builtins.elemAt parts 0;
        name = builtins.elemAt parts 1;
      }
    else
      null;

  resolvesAs = resources: expectedType: ref:
    let parsed = parseRef ref;
    in parsed != null
    && parsed.type == expectedType
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
    && !(lib.hasInfix "\0" value)
    && !(lib.hasInfix ":" value)
    && !(builtins.elem ".." (lib.splitString "/" value))
    && (value == "" || lib.all
      (component: component != "" && component != ".")
      (lib.splitString "/" value))
    && !(lib.any
      (separator: lib.hasInfix separator value)
      [ "⁄" "∕" "⧸" "⫸" "／" "＼" "﹨" "．" "․" ]);

  # A guest-side mount path is absolute and still carries no traversal.
  guestMountPath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && builtins.stringLength value <= maxLayoutPathBytes
    && !(lib.hasInfix "\\" value)
    && !(lib.hasInfix "\0" value)
    && !(lib.any
      (separator: lib.hasInfix separator value)
      [ "⁄" "∕" "⧸" "⫸" "／" "＼" "﹨" "．" "․" ])
    && (value == "/" || lib.all
      (component: component != "" && component != "." && component != "..")
      (lib.tail (lib.splitString "/" value)));

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  aclAssertions = path: resources: entryIndex: field: grants:
    let safeGrants = if builtins.isList grants then grants else [ ];
    in [
      {
        assertion = builtins.isList grants && lib.length grants <= 64;
        message = "${path}.layout.${toString entryIndex}.${field} must contain at most 64 grants.";
      }
    ] ++ lib.flatten (lib.imap0
      (grantIndex: grant:
        let
          where = "${path}.layout.${toString entryIndex}.${field}.${toString grantIndex}";
          permissionValue = attrOr grant "permissions" "";
        in [
          {
            assertion =
              builtins.isAttrs grant
              && exactKeys [ "principal" "permissions" ] grant
              && builtins.hasAttr "principal" grant
              && builtins.isAttrs grant.principal
              && builtins.hasAttr "ref" grant.principal
              && resolvesAs resources "User" grant.principal.ref;
            message = "${where}.principal.ref must be a User in the same Zone; a numeric identity is not accepted.";
          }
          {
            assertion = builtins.isString permissionValue
              && builtins.match permissionsPattern permissionValue != null;
            message = "${where}.permissions must be a POSIX rwx string.";
          }
        ])
      safeGrants);

  layoutAssertions = path: resources: layout:
    let
      safeLayout = if builtins.isList layout then layout else [ ];
      layoutPaths = map (entry: attrOr entry "path" null) safeLayout;
    in [
      {
        assertion = builtins.isList layout;
        message = "${path}.layout must be a list of LayoutEntry objects.";
      }
      {
        assertion = builtins.isList layout && lib.length layout <= maxLayoutEntries;
        message = "${path}.layout must contain at most ${toString maxLayoutEntries} entries.";
      }
      {
        assertion = builtins.isList layout
          && lib.length (lib.unique layoutPaths) == lib.length layout;
        message = "${path}.layout entries must declare unique anchored paths.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: entry:
        let
          where = "${path}.layout.${toString index}";
          entryType = attrOr entry "type" null;
          target = attrOr entry "target" null;
          invariants = attrOr entry "invariants" [ ];
          safeInvariants = if builtins.isList invariants then invariants else [ ];
          recursiveValue = attrOr entry "recursive" false;
          recursive = if builtins.isBool recursiveValue then recursiveValue else false;
          modeValue = attrOr entry "mode" "";
        in
        [
          {
            assertion = exactKeys layoutKeys entry;
            message = "${where} contains an unsupported layout field.";
          }
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
            assertion = builtins.isString modeValue
              && builtins.match modePattern modeValue != null;
            message = "${where}.mode must be a four-digit octal string.";
          }
          {
            assertion = entryType != "symlink" -> target == null;
            message = "${where}.target is accepted only for a symlink entry. Remove target, or set ${where}.type to symlink.";
          }
          {
            assertion = entryType == "symlink"
              -> (target != null && target != "" && anchoredPath target);
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
          {
            assertion = builtins.elem (attrOr entry "sensitivity" "private")
              [ "public" "private" "secret-adjacent" "audit" "zone-scoped" "secret" ];
            message = "${where}.sensitivity is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "createPolicy" "create-if-absent")
              [
                "create-if-absent" "create-if-never-provisioned"
                "always-recreate" "observe-only"
              ];
            message = "${where}.createPolicy is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "repairPolicy" "exact-owner")
              [
                "none" "nix-activation" "exact-owner" "fail-closed"
                "operator-only" "exact-mode" "exact-owner-and-acl"
              ];
            message = "${where}.repairPolicy is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "cleanupPolicy" "never")
              [
                "never" "boot" "process-exit-with-proof" "vm-stop-with-proof"
                "cutover-only" "owner-controlled" "external" "process-exit"
              ];
            message = "${where}.cleanupPolicy is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "adoptionPolicy" "adopt-with-live-owner-proof")
              [
                "adopt-with-live-owner-proof" "recreate-from-persistent"
                "quarantine-on-ambiguity" "delete-if-owner-dead" "not-adoptable"
                "never-adopt"
              ];
            message = "${where}.adoptionPolicy is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "restartPolicy" "preserve-across-controller-restart")
              [
                "preserve-across-controller-restart" "recreate-after-owner-death"
                "cleanup-after-owner-death" "manual-recovery" "not-applicable"
                "recreate-on-controller-restart"
              ];
            message = "${where}.restartPolicy is invalid.";
          }
          {
            assertion = builtins.elem (attrOr entry "leaseClass" "none")
              [ "none" "process-pidfd" "cgroup-leaf" "file-record" "external" "controller-lock" ];
            message = "${where}.leaseClass is invalid.";
          }
          {
            assertion = !(builtins.elem (attrOr entry "cleanupPolicy" "never")
              [ "process-exit-with-proof" "process-exit" ])
              || builtins.elem (attrOr entry "leaseClass" "none")
                [ "process-pidfd" "cgroup-leaf" ];
            message = "${where}.process cleanup requires a pidfd or cgroup lease.";
          }
          {
            assertion = (attrOr entry "cleanupPolicy" "never") != "vm-stop-with-proof"
              || (attrOr entry "leaseClass" "none") == "cgroup-leaf";
            message = "${where}.vm-stop cleanup requires a cgroup lease.";
          }
          {
            assertion = (attrOr entry "leaseClass" "none") != "file-record"
              || ((attrOr entry "type" null) == "file"
                && (attrOr entry "cleanupPolicy" "never") == "never");
            message = "${where}.file-record leases require a never-cleaned regular file.";
          }
          {
            assertion = (attrOr entry "createPolicy" "create-if-absent") != "always-recreate"
              || (
                builtins.elem (attrOr entry "leaseClass" "none") [ "process-pidfd" "cgroup-leaf" ]
                && builtins.elem (attrOr entry "cleanupPolicy" "never")
                  [ "process-exit-with-proof" "process-exit" ]
              );
            message = "${where}.always-recreate requires a process lease and process cleanup.";
          }
          {
            assertion = builtins.isList invariants
              && lib.length (lib.unique safeInvariants) == lib.length safeInvariants;
            message = "${where}.invariants must be a unique list.";
          }
          {
            assertion = builtins.isList invariants
              && lib.all
                (invariant: builtins.elem invariant [
                  "no-symlink" "no-magic-link" "no-recursive-mutation"
                  "same-filesystem" "hardlink-farm-no-recursion"
                  "broker-opaque-id-only" "root-owned-parent"
                  "scope-authorization-required"
                ])
                safeInvariants;
            message = "${where}.invariants contains an unknown value.";
          }
          {
            assertion = builtins.isBool recursiveValue;
            message = "${where}.recursive must be boolean.";
          }
          {
            assertion = !recursive
              || builtins.elem (attrOr entry "repairPolicy" "exact-owner")
                [ "exact-owner" "fail-closed" "exact-owner-and-acl" ];
            message = "${where}.recursive requires exact-owner or fail-closed repair.";
          }
          {
            assertion = !recursive
              || !(builtins.elem "no-recursive-mutation" safeInvariants
                || builtins.elem "hardlink-farm-no-recursion" safeInvariants);
            message = "${where}.recursive conflicts with no-recursive-mutation or hardlink-farm-no-recursion.";
          }
        ]
        ++ aclAssertions path resources index "accessAcl" (attrOr entry "accessAcl" [ ])
        ++ aclAssertions path resources index "defaultAcl" (attrOr entry "defaultAcl" [ ]))
      safeLayout);

  viewAssertions = path: views:
    let safeViews = if builtins.isAttrs views then views else { };
    in [
      {
        assertion = builtins.isAttrs views;
        message = "${path}.views must be an attribute set of named views.";
      }
      {
        assertion = builtins.isAttrs views && views != { };
        message = "${path}.views must declare at least one named view.";
      }
      {
        assertion = builtins.isAttrs views && lib.length (builtins.attrNames views) <= maxViews;
        message = "${path}.views must contain at most ${toString maxViews} views.";
      }
    ]
    ++ lib.flatten (lib.mapAttrsToList
      (name: view:
        let
          where = "${path}.views.${name}";
          rightsValue = attrOr view "rights" [ ];
          rights = if builtins.isList rightsValue then rightsValue else [ ];
        in [
          {
            assertion = exactKeys viewKeys view;
            message = "${where} contains an unsupported view field.";
          }
          {
            assertion = builtins.match tokenPattern name != null;
            message = "${where}: view name must match ${tokenPattern}.";
          }
          {
            assertion = anchoredPath (attrOr view "path" null);
            message = "${where}.path must be anchored inside the Volume.";
          }
          {
            assertion = builtins.isList rightsValue
              && rights != [ ]
              && lib.length (lib.unique rights) == lib.length rights
              && lib.all
                (right: builtins.elem right [ "read" "write" "create" "delete" "traverse" "execute" ])
                rights;
            message = "${where}.rights must be a non-empty set of unique known rights.";
          }
        ])
      safeViews);

  attachmentAssertions = path: resources: views: sourceKind: attachments:
    let
      safeViews = if builtins.isAttrs views then views else { };
      safeAttachments = if builtins.isList attachments then attachments else [ ];
      writers = lib.filter (a: attrOr a "access" "read-only" == "read-write") safeAttachments;
      executionRefs = map (attachment: attrOr attachment "executionRef" null) safeAttachments;
    in
    [
      {
        assertion = builtins.isList attachments;
        message = "${path}.attachments must be a list of Attachment objects.";
      }
      {
        assertion = builtins.isList attachments && lib.length attachments <= maxAttachments;
        message = "${path}.attachments must contain at most ${toString maxAttachments} attachments.";
      }
      {
        assertion = lib.length writers <= 1;
        message = "${path}.attachments may declare at most one read-write attachment. Set every other attachment's access to read-only or shared-write.";
      }
      {
        assertion = builtins.isList attachments
          && lib.length (lib.unique executionRefs) == lib.length executionRefs;
        message = "${path}.attachments must target each executionRef at most once.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: attachment:
        let
          where = "${path}.attachments.${toString index}";
          view = attrOr attachment "view" null;
          access = attrOr attachment "access" "read-only";
          rightsValue = attrOr (attrOr safeViews view { }) "rights" [ ];
          rights = if builtins.isList rightsValue then rightsValue else [ ];
          settings = attrOr attachment "settings" { };
          posixAcl = attrOr settings "posixAcl" false;
          xattr = attrOr settings "xattr" false;
          cache = attrOr settings "cache" "auto";
          inodeFileHandles = attrOr settings "inodeFileHandles" "never";
          threadPoolSize = attrOr settings "threadPoolSize" null;
          socketGroup = attrOr settings "socketGroup" null;
        in
        [
          {
            assertion = exactKeys attachmentKeys attachment;
            message = "${where} contains an unsupported attachment field.";
          }
          {
            assertion = exactKeys attachmentSettingKeys settings;
            message = "${where}.settings contains an unsupported field.";
          }
          {
            assertion = builtins.isBool posixAcl;
            message = "${where}.settings.posixAcl must be boolean.";
          }
          {
            assertion = builtins.isBool xattr;
            message = "${where}.settings.xattr must be boolean.";
          }
          {
            assertion = builtins.elem cache [ "auto" "always" "never" ];
            message = "${where}.settings.cache must be auto, always, or never.";
          }
          {
            assertion = builtins.elem inodeFileHandles [ "never" "prefer" "mandatory" ];
            message = "${where}.settings.inodeFileHandles must be never, prefer, or mandatory.";
          }
          {
            assertion = isExecutionRef resources (attrOr attachment "executionRef" "");
            message = "${where}.executionRef must resolve to a Host or Guest in the same Zone.";
          }
          {
            assertion = builtins.elem (attrOr attachment "transport" null) [ "virtiofs" "virtio-blk" ];
            message = "${where}.transport must be virtiofs or virtio-blk.";
          }
          {
            assertion =
              sourceKind != "block-image"
              || attrOr attachment "transport" null == "virtio-blk";
            message = "${where}.transport must be virtio-blk for a block-image Volume.";
          }
          {
            assertion =
              sourceKind == "block-image"
              || attrOr attachment "transport" null != "virtio-blk";
            message = "${where}.transport virtio-blk is accepted only for a block-image Volume.";
          }
          {
            assertion = sourceKind == "block-image"
              || attrOr attachment "transport" null == "virtiofs";
            message = "${where}.transport must be virtiofs unless the Volume is block-image.";
          }
          {
            assertion = view != null && builtins.hasAttr view safeViews;
            message = "${where}.view must name a view the Volume declares.";
          }
          {
            assertion = builtins.elem access [ "read-only" "read-write" "shared-write" ];
            message = "${where}.access must be read-only, read-write, or shared-write.";
          }
          {
            assertion = access == "read-only"
              || (builtins.isList rightsValue && builtins.elem "write" rights);
            message = "${where}.access requires the selected view to grant the write right. Add write to that view's rights under ${path}.views, or set ${where}.access to read-only.";
          }
          {
            assertion = guestMountPath (attrOr attachment "mountPath" null);
            message = "${where}.mountPath must be an absolute guest-side path with no '..' component.";
          }
          {
            assertion = threadPoolSize == null
              || (builtins.isInt threadPoolSize && threadPoolSize >= 1 && threadPoolSize <= 256);
            message = "${where}.settings.threadPoolSize must be null or an integer from 1 to 256.";
          }
          {
            assertion = socketGroup == null
              || (builtins.isString socketGroup && builtins.match tokenPattern socketGroup != null);
            message = "${where}.settings.socketGroup must be null or a bounded token.";
          }
        ])
      safeAttachments);

  sourceAssertions = path: resources: source: quota: volumeKind: layout:
    let
      safeSource = if builtins.isAttrs source then source else { };
      settings = attrOr source "settings" { };
      safeSettings = if builtins.isAttrs settings then settings else { };
      kind = attrOr safeSettings "kind" null;
      policyId = attrOr safeSettings "sourcePolicyId" null;
      imageFormat = attrOr safeSettings "imageFormat" null;
      preallocate = attrOr safeSettings "preallocate" false;
      hostBacked = builtins.elem kind [ "local-path" "block-image" ];
      maxBytes = attrOr quota "maxBytes" null;
      maxInodes = attrOr quota "maxInodes" null;
      enforcement = attrOr quota "enforcement" "none";
    in
    [
      {
        assertion = builtins.isAttrs source;
        message = "${path}.source must be an attribute set.";
      }
      {
        assertion = exactKeys [ "executionRef" "settings" ] safeSource;
        message = "${path}.source contains an unsupported field.";
      }
      {
        assertion = builtins.isAttrs settings && exactKeys sourceSettingKeys safeSettings;
        message = "${path}.source.settings contains an unsupported field.";
      }
      {
        assertion = quota == null || builtins.isAttrs quota;
        message = "${path}.quota must be null or an attribute set.";
      }
      {
        assertion = quota == null || exactKeys quotaKeys quota;
        message = "${path}.quota contains an unsupported field.";
      }
      {
        assertion = isExecutionRef resources (attrOr source "executionRef" "");
        message = "${path}.source.executionRef must resolve to a Host or Guest in the same Zone.";
      }
      {
        assertion = builtins.elem kind [ "local-path" "block-image" "tmpfs" ];
        message = "${path}.source.settings.kind must be local-path, block-image, or tmpfs.";
      }
      {
        assertion = !(builtins.hasAttr "path" safeSettings)
          && !(builtins.hasAttr "hostPath" safeSettings);
        message = "${path}.source.settings must not carry a host path; a Volume source is an opaque policy ID. Remove the path and hostPath keys and name the root through ${path}.source.settings.sourcePolicyId instead.";
      }
      {
        assertion = hostBacked
          -> (builtins.isString policyId && builtins.match tokenPattern policyId != null);
        message = "${path}.source.settings.sourcePolicyId is required for a host-backed source and must match ${tokenPattern}.";
      }
      {
        assertion = !hostBacked -> policyId == null;
        message = "${path}.source.settings.sourcePolicyId is accepted only for a host-backed source. Remove sourcePolicyId, or set ${path}.source.settings.kind to local-path or block-image.";
      }
      {
        assertion = kind != "block-image"
          || imageFormat == null
          || builtins.elem imageFormat [ "raw" "qcow2" ];
        message = "${path}.source.settings.imageFormat must be raw or qcow2 for a block-image source.";
      }
      {
        assertion = kind == "block-image" || imageFormat == null;
        message = "${path}.source.settings.imageFormat is accepted only for a block-image source.";
      }
      {
        assertion = builtins.isBool preallocate;
        message = "${path}.source.settings.preallocate must be boolean.";
      }
      {
        assertion = kind == "block-image" || preallocate == false;
        message = "${path}.source.settings.preallocate is accepted only for a block-image source.";
      }
      {
        assertion = kind != "block-image" || maxBytes != null;
        message = "${path}.quota.maxBytes is required for a block-image source.";
      }
      {
        assertion = kind != "block-image" || builtins.elem volumeKind [ "durable" "ephemeral" ];
        message = "${path}.kind must be durable or ephemeral for a block-image source.";
      }
      {
        assertion = kind != "tmpfs" || (maxBytes != null && maxInodes != null);
        message = "${path}.quota.maxBytes and ${path}.quota.maxInodes are required for a tmpfs source.";
      }
      {
        assertion = kind != "tmpfs" || enforcement == "hard";
        message = "${path}.quota.enforcement must be hard for a tmpfs source.";
      }
      {
        assertion = kind != "tmpfs" || builtins.elem volumeKind [ "ephemeral" "tmp" ];
        message = "${path}.kind must be ephemeral or tmp for a tmpfs source.";
      }
      {
        assertion = quota == null
          || builtins.elem enforcement [ "none" "hard" ];
        message = "${path}.quota.enforcement must be none or hard.";
      }
      {
        assertion = quota == null
          || (maxBytes == null || (builtins.isInt maxBytes && maxBytes > 0));
        message = "${path}.quota.maxBytes must be a positive integer when present.";
      }
      {
        assertion = quota == null
          || (maxInodes == null || (builtins.isInt maxInodes && maxInodes > 0));
        message = "${path}.quota.maxInodes must be a positive integer when present.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: entry:
        let
          where = "${path}.layout.${toString index}";
          createPolicy = attrOr entry "createPolicy" "create-if-absent";
          restartPolicy = attrOr entry "restartPolicy" "preserve-across-controller-restart";
        in lib.optionals (kind == "tmpfs") [
          {
            assertion = createPolicy != "create-if-never-provisioned";
            message = "${where}.createPolicy is not valid for a tmpfs source; readiness cannot depend on prior provisioning.";
          }
          {
            assertion = restartPolicy != "preserve-across-controller-restart";
            message = "${where}.restartPolicy is not valid for a tmpfs source; readiness cannot depend on controller restart persistence.";
          }
        ])
      (if builtins.isList layout then layout else [ ]));

  volumeAssertions = zoneName: resourceName: resources: resource:
    let
      path = "d2b.zones.${zoneName}.resources.${resourceName}";
      spec = attrOr resource "spec" { };
      safeSpec = if builtins.isAttrs spec then spec else { };
      views = attrOr safeSpec "views" { };
    in
    [
      {
        assertion = builtins.isAttrs resource && builtins.isAttrs spec;
        message = "${path} and its spec must be attribute sets.";
      }
      {
        assertion = resolvesAs resources "Provider" (attrOr safeSpec "providerRef" "");
        message = "${path}.spec.providerRef must resolve to a Provider in Zone ${zoneName}.";
      }
      {
        assertion =
          let
            providerRef = parseRef (attrOr safeSpec "providerRef" "");
            provider =
              if providerRef != null && builtins.hasAttr providerRef.name resources
              then resources.${providerRef.name}
              else null;
            artifactId =
              if provider != null && builtins.isAttrs (attrOr provider "spec" null)
                && builtins.hasAttr "artifactId" provider.spec
              then provider.spec.artifactId
              else null;
            artifact =
              if artifactId != null && builtins.hasAttr artifactId cfg.artifacts
              then cfg.artifacts.${artifactId}
              else null;
          in provider != null
            && artifactId != null
            && artifact != null
            && artifact.type == "provider";
        message = "${path}.spec.providerRef must select a Provider with a provider artifact in d2b.artifacts.";
      }
      {
        assertion = builtins.elem (attrOr safeSpec "kind" null) [ "durable" "ephemeral" "state" "tmp" "cache" ];
        message = "${path}.spec.kind must be durable, ephemeral, state, tmp, or cache.";
      }
    ]
    ++ sourceAssertions "${path}.spec" resources (attrOr safeSpec "source" { }) (attrOr safeSpec "quota" { })
      (attrOr safeSpec "kind" null) (attrOr safeSpec "layout" [ ])
    ++ layoutAssertions "${path}.spec" resources (attrOr safeSpec "layout" [ ])
    ++ viewAssertions "${path}.spec" views
    ++ attachmentAssertions "${path}.spec" resources views
      (attrOr (attrOr (attrOr safeSpec "source" { }) "settings" { }) "kind" null)
      (attrOr safeSpec "attachments" [ ]);

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
  config = {
    assertions = zoneVolumeAssertions;
    # Keep the Volume validator's canonical assertion records available to
    # module-level consumers without forcing the bundle integrity gate. The
    # latter deliberately rejects path-shaped Volume fields by throwing.
    d2b._resourceCompiler.volumeValidation = zoneVolumeAssertions;
  };
}
