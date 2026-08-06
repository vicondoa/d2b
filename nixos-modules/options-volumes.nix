# User-facing Volume base option types.
#
# The resource compiler remains the source of canonical defaults and bundle
# projection.  These declarations give the unified Zone resource surface
# typed LayoutEntry, ViewSpec, and Attachment values without adding a second
# volume namespace or materialising Volume-only defaults on other resources.
{ config, lib, ... }:

let
  cfg = config.d2b;
  types = lib.types;

  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  resourceNamePattern = tokenPattern;
  resourceRefPattern =
    "^([A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$";
  userRefPattern = "^User/[a-z][a-z0-9-]{0,62}$";
  anchoredPath = value:
    builtins.isString value
    && builtins.stringLength value <= 255
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
  guestPath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && builtins.stringLength value <= 255
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

  anchoredPathType = types.addCheck types.str anchoredPath;
  guestPathType = types.addCheck types.str guestPath;
  resourceRefType = types.addCheck types.str
    (value: builtins.match resourceRefPattern value != null);
  userRefType = types.addCheck types.str
    (value: builtins.match userRefPattern value != null);

  aclGrantType = types.submodule {
    freeformType = null;
    options = {
      principal = lib.mkOption {
        type = types.submodule {
          freeformType = null;
          options.ref = lib.mkOption {
            type = userRefType;
            description = "Same-Zone User resource that receives this ACL grant.";
          };
        };
      };
      permissions = lib.mkOption {
        type = types.strMatching "^[rwx]{1,3}$";
        description = "POSIX rwx permissions for the named User.";
      };
    };
  };

  layoutEntryType = types.submodule {
    freeformType = null;
    options = {
      path = lib.mkOption {
        type = anchoredPathType;
        description = "Path anchored below the Volume root; the empty path is the root.";
      };
      type = lib.mkOption {
        type = types.enum [ "directory" "file" "symlink" "unix-socket" ];
      };
      ownerRef = lib.mkOption {
        type = userRefType;
        description = "Same-Zone User resource owning the entry.";
      };
      groupRef = lib.mkOption {
        type = userRefType;
        description = "Same-Zone User resource owning the entry's group.";
      };
      mode = lib.mkOption {
        type = types.strMatching "^[0-7]{4}$";
      };
      target = lib.mkOption {
        type = types.nullOr anchoredPathType;
        default = null;
        description = "Anchored target required only for symlink entries.";
      };
      accessAcl = lib.mkOption {
        type = types.listOf aclGrantType;
        default = [ ];
      };
      defaultAcl = lib.mkOption {
        type = types.listOf aclGrantType;
        default = [ ];
      };
      foreignChildPolicy = lib.mkOption {
        type = types.enum [ "preserve" "fail" ];
        default = "preserve";
      };
      noFollow = lib.mkOption {
        type = types.bool;
        default = true;
      };
      recursive = lib.mkOption {
        type = types.bool;
        default = false;
      };
      sensitivity = lib.mkOption {
        type = types.enum [
          "public"
          "private"
          "secret-adjacent"
          "audit"
          "zone-scoped"
          "secret"
        ];
        default = "private";
      };
      createPolicy = lib.mkOption {
        type = types.enum [
          "create-if-absent"
          "create-if-never-provisioned"
          "always-recreate"
          "observe-only"
        ];
        default = "create-if-absent";
      };
      repairPolicy = lib.mkOption {
        type = types.enum [
          "none"
          "nix-activation"
          "exact-owner"
          "fail-closed"
          "operator-only"
          "exact-mode"
          "exact-owner-and-acl"
        ];
        default = "exact-owner";
      };
      cleanupPolicy = lib.mkOption {
        type = types.enum [
          "never"
          "boot"
          "process-exit-with-proof"
          "vm-stop-with-proof"
          "cutover-only"
          "external"
          "owner-controlled"
          "process-exit"
        ];
        default = "never";
      };
      adoptionPolicy = lib.mkOption {
        type = types.enum [
          "adopt-with-live-owner-proof"
          "recreate-from-persistent"
          "quarantine-on-ambiguity"
          "delete-if-owner-dead"
          "not-adoptable"
          "never-adopt"
        ];
        default = "adopt-with-live-owner-proof";
      };
      restartPolicy = lib.mkOption {
        type = types.enum [
          "preserve-across-controller-restart"
          "recreate-after-owner-death"
          "cleanup-after-owner-death"
          "manual-recovery"
          "not-applicable"
          "recreate-on-controller-restart"
        ];
        default = "preserve-across-controller-restart";
      };
      leaseClass = lib.mkOption {
        type = types.enum [
          "none"
          "process-pidfd"
          "cgroup-leaf"
          "file-record"
          "external"
          "controller-lock"
        ];
        default = "none";
      };
      invariants = lib.mkOption {
        type = types.listOf (types.enum [
          "no-symlink"
          "no-magic-link"
          "no-recursive-mutation"
          "same-filesystem"
          "hardlink-farm-no-recursion"
          "broker-opaque-id-only"
          "root-owned-parent"
          "scope-authorization-required"
        ]);
        default = [ "no-symlink" ];
      };
    };
  };

  viewType = types.submodule {
    freeformType = null;
    options = {
      path = lib.mkOption {
        type = anchoredPathType;
      };
      rights = lib.mkOption {
        type = types.listOf (types.enum [
          "read"
          "write"
          "create"
          "delete"
          "traverse"
          "execute"
        ]);
        default = [ ];
      };
    };
  };

  attachmentSettingsType = types.submodule {
    freeformType = null;
    options = {
      posixAcl = lib.mkOption {
        type = types.bool;
        default = false;
      };
      xattr = lib.mkOption {
        type = types.bool;
        default = false;
      };
      cache = lib.mkOption {
        type = types.enum [ "auto" "always" "never" ];
        default = "auto";
      };
      inodeFileHandles = lib.mkOption {
        type = types.enum [ "never" "prefer" "mandatory" ];
        default = "never";
      };
      threadPoolSize = lib.mkOption {
        type = types.nullOr (types.ints.between 1 256);
        default = null;
      };
      socketGroup = lib.mkOption {
        type = types.nullOr (types.strMatching tokenPattern);
        default = null;
      };
    };
  };

  attachmentType = types.submodule {
    freeformType = null;
    options = {
      executionRef = lib.mkOption {
        type = types.oneOf [ (types.strMatching "^Host/[a-z][a-z0-9-]{0,62}$") (types.strMatching "^Guest/[a-z][a-z0-9-]{0,62}$") ];
      };
      transport = lib.mkOption {
        type = types.enum [ "virtiofs" "virtio-blk" ];
      };
      view = lib.mkOption {
        type = types.strMatching tokenPattern;
      };
      access = lib.mkOption {
        type = types.enum [ "read-only" "read-write" "shared-write" ];
        default = "read-only";
      };
      mountPath = lib.mkOption {
        type = guestPathType;
      };
      settings = lib.mkOption {
        type = attachmentSettingsType;
        default = { };
      };
    };
  };

  sourceSettingsType = types.submodule {
    freeformType = null;
    options = {
      kind = lib.mkOption {
        type = types.enum [ "local-path" "block-image" "tmpfs" ];
      };
      sourcePolicyId = lib.mkOption {
        type = types.nullOr (types.strMatching tokenPattern);
        default = null;
      };
      imageFormat = lib.mkOption {
        type = types.nullOr (types.enum [ "raw" "qcow2" ]);
        default = null;
      };
      preallocate = lib.mkOption {
        type = types.bool;
        default = false;
      };
    };
  };

  sourceType = types.submodule {
    freeformType = null;
    options = {
      executionRef = lib.mkOption {
        type = types.oneOf [ (types.strMatching "^Host/[a-z][a-z0-9-]{0,62}$") (types.strMatching "^Guest/[a-z][a-z0-9-]{0,62}$") ];
      };
      settings = lib.mkOption {
        type = sourceSettingsType;
      };
    };
  };

  quotaType = types.submodule {
    freeformType = null;
    options = {
      maxBytes = lib.mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
      };
      maxInodes = lib.mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
      };
      enforcement = lib.mkOption {
        type = types.enum [ "none" "hard" ];
        default = "none";
      };
    };
  };

  providerExtensionType = types.submodule {
    freeformType = null;
    options = {
      schemaId = lib.mkOption {
        type = types.strMatching "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Za-z][A-Za-z0-9-]{0,62}$";
      };
      schemaVersion = lib.mkOption {
        type = types.strMatching "^[0-9]+\\.[0-9]+$";
      };
      settings = lib.mkOption {
        type = types.attrsOf types.unspecified;
        default = { };
      };
    };
  };

  updatePolicyType = types.submodule {
    freeformType = null;
    options.mode = lib.mkOption {
      type = types.enum [ "manual" "automatic" "manual-disruptive" ];
      default = "manual";
    };
  };

  volumeSpecType = types.submodule {
    freeformType = types.attrsOf types.unspecified;
    options = {
      providerRef = lib.mkOption {
        type = types.strMatching resourceRefPattern;
      };
      source = lib.mkOption {
        type = sourceType;
      };
      kind = lib.mkOption {
        type = types.enum [ "durable" "ephemeral" "state" "tmp" "cache" ];
      };
      layout = lib.mkOption {
        type = types.listOf layoutEntryType;
        default = [ ];
      };
      views = lib.mkOption {
        type = types.attrsOf viewType;
        default = { };
      };
      attachments = lib.mkOption {
        type = types.listOf attachmentType;
        default = [ ];
      };
      quota = lib.mkOption {
        type = types.nullOr quotaType;
        default = null;
      };
      provider = lib.mkOption {
        type = types.nullOr providerExtensionType;
        default = null;
      };
      updatePolicy = lib.mkOption {
        type = updatePolicyType;
        default = { };
      };
    };
  };

  volumeOptionTypes = {
    inherit
      aclGrantType
      attachmentSettingsType
      attachmentType
      guestPathType
      layoutEntryType
      quotaType
      sourceSettingsType
      sourceType
      viewType
      volumeSpecType
      ;
    anchoredPath = anchoredPath;
  };

  volumeRows = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          spec = resource.spec or { };
        })
        (lib.filterAttrs (_: resource: resource.type == "Volume") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  guestRows = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          spec = resource.spec or { };
        })
        (lib.filterAttrs (_: resource: resource.type == "Guest") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  typedResource = type: name: zoneName: spec: ownerRef:
    {
      inherit type;
      metadata = {
        name = name;
        zone = zoneName;
      } // lib.optionalAttrs (ownerRef != null) { inherit ownerRef; };
      inherit spec;
    };

  storeViewLayout = guestName:
    [
      {
        path = "";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0755";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "never";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [ "no-symlink" "scope-authorization-required" ];
        target = null;
      }
      {
        path = "live";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0755";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "cutover-only";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [ "no-symlink" "broker-opaque-id-only" ];
        target = null;
      }
      {
        path = "live/.d2b-marker-${guestName}";
        type = "file";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0444";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "cutover-only";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [
          "no-symlink"
          "same-filesystem"
          "hardlink-farm-no-recursion"
          "broker-opaque-id-only"
        ];
        target = null;
      }
      {
        path = "meta";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0755";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "never";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [
          "no-symlink"
          "same-filesystem"
          "hardlink-farm-no-recursion"
          "broker-opaque-id-only"
        ];
        target = null;
      }
      {
        path = "meta/generations";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0755";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "cutover-only";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [
          "no-symlink"
          "same-filesystem"
          "hardlink-farm-no-recursion"
          "broker-opaque-id-only"
        ];
        target = null;
      }
      {
        path = "meta/current";
        type = "symlink";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0777";
        target = "generations/0";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = false;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "cutover-only";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [ "broker-opaque-id-only" ];
      }
      {
        path = "state";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0700";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "never";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [ "no-symlink" "broker-opaque-id-only" ];
        target = null;
      }
      {
        path = "gcroots";
        type = "directory";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0755";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "cutover-only";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [
          "no-symlink"
          "same-filesystem"
          "hardlink-farm-no-recursion"
          "broker-opaque-id-only"
        ];
        target = null;
      }
      {
        path = "sync.lock";
        type = "file";
        ownerRef = "User/d2bd";
        groupRef = "User/users";
        mode = "0640";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "private";
        createPolicy = "create-if-absent";
        repairPolicy = "exact-owner";
        cleanupPolicy = "never";
        adoptionPolicy = "adopt-with-live-owner-proof";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "file-record";
        invariants = [ "no-symlink" "broker-opaque-id-only" ];
        target = null;
      }
    ];

  defaultAttachmentSettings = {
    posixAcl = false;
    xattr = false;
    cache = "auto";
    inodeFileHandles = "never";
    threadPoolSize = null;
    socketGroup = null;
  };

  storeViewVolume = guest:
    let
      guestName = guest.resourceName;
    in
    typedResource "Volume" "store-view-${guestName}" guest.zoneName {
      providerRef = "Provider/volume-local";
      source = {
        executionRef = "Host/host-system";
        settings = {
          kind = "local-path";
          sourcePolicyId = "state-root";
          imageFormat = null;
          preallocate = false;
        };
      };
      kind = "durable";
      layout = storeViewLayout guestName;
      views = {
        ro-store = {
          path = "live";
          rights = [ "read" "traverse" ];
        };
        meta = {
          path = "meta";
          rights = [ "read" "traverse" ];
        };
      };
      attachments = [{
        executionRef = "Guest/${guestName}";
        transport = "virtiofs";
        view = "ro-store";
        access = "read-only";
        mountPath = "/nix/.ro-store";
        settings = defaultAttachmentSettings;
      }];
      quota = null;
    } "Guest/${guestName}";

  tpmEnabled = guest:
    let spec = guest.spec;
    in (attrOr spec "tpmEnabled" false)
      || ((attrOr spec "tpm" { }).enable or false);

  tpmVolume = guest:
    let
      guestName = guest.resourceName;
      owner = "User/d2b-${guestName}-swtpm";
      root = {
        path = "";
        type = "directory";
        ownerRef = owner;
        groupRef = owner;
        mode = "0700";
        accessAcl = [ ];
        defaultAcl = [ ];
        foreignChildPolicy = "preserve";
        noFollow = true;
        recursive = false;
        sensitivity = "secret-adjacent";
        createPolicy = "create-if-never-provisioned";
        repairPolicy = "fail-closed";
        cleanupPolicy = "never";
        adoptionPolicy = "quarantine-on-ambiguity";
        restartPolicy = "preserve-across-controller-restart";
        leaseClass = "none";
        invariants = [
          "no-symlink"
          "broker-opaque-id-only"
          "root-owned-parent"
          "scope-authorization-required"
        ];
        target = null;
      };
    in
    typedResource "Volume" "swtpm-${guestName}" guest.zoneName {
      providerRef = "Provider/volume-local";
      source = {
        executionRef = "Host/host-system";
        settings = {
          kind = "local-path";
          sourcePolicyId = "state-root";
          imageFormat = null;
          preallocate = false;
        };
      };
      kind = "state";
      layout = [ root ];
      views = {
        controller = {
          path = "";
          rights = [ "read" "write" "create" "delete" "traverse" ];
        };
      };
      attachments = [ ];
      quota = null;
    } "Guest/${guestName}";

  generatedGuests = lib.concatMap
    (guest: [ (storeViewVolume guest) ]
      ++ lib.optional (tpmEnabled guest) (tpmVolume guest))
    guestRows;

  volumeAttachmentRows = lib.concatMap
    (row:
      lib.imap0
        (index: attachment: {
          zoneName = row.zoneName;
          volumeName = row.resourceName;
          index = index;
          attachment = attachment;
        })
        (attrOr row.spec "attachments" [ ]))
    volumeRows;

  generatedVfdUsersRaw = lib.map
    (row:
      let
        volumeName = row.volumeName;
        userName = "vol-${volumeName}-vfd";
      in {
        zoneName = row.zoneName;
        name = userName;
        resource = typedResource "User" userName row.zoneName { } null;
      })
    (lib.filter (row: (attrOr row.attachment "transport" null) == "virtiofs")
      volumeAttachmentRows);

  generatedVfdUsersByKey = lib.foldl'
    (result: row:
      result // {
        "${row.zoneName}/${row.name}" = row;
      })
    { }
    generatedVfdUsersRaw;
  generatedVfdUsers = lib.attrValues generatedVfdUsersByKey;

  generatedVirtiofsProvider =
    lib.unique (map (row: row.zoneName)
      (lib.filter (row: (attrOr row.attachment "transport" null) == "virtiofs")
        volumeAttachmentRows ++ lib.concatMap
          (guest: [{
            zoneName = guest.zoneName;
            attachment = { transport = "virtiofs"; };
          }])
          guestRows));

  generatedProviderResources = lib.concatMap
    (zoneName:
      let
        authored = cfg.zones.${zoneName}.resources or { };
        hasLocal = lib.any (row: row.metadata.zone == zoneName) generatedGuests
          && !(builtins.hasAttr "volume-local" authored);
        hasVirtiofs = builtins.elem zoneName generatedVirtiofsProvider
          && !(builtins.hasAttr "volume-virtiofs" authored);
      in
      lib.optional hasLocal {
        zoneName = zoneName;
        name = "volume-local";
        resource = typedResource "Provider" "volume-local" zoneName {
          artifactId = "volume-local-provider";
          config = { };
        } null;
      }
      ++ lib.optional hasVirtiofs {
        zoneName = zoneName;
        name = "volume-virtiofs";
        resource = typedResource "Provider" "volume-virtiofs" zoneName {
          artifactId = "volume-virtiofs-provider";
          config = { };
        } null;
      })
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  generatedRows = (map (resource: {
      zoneName = resource.metadata.zone;
      name = resource.metadata.name;
      inherit resource;
    }) generatedGuests)
    ++ generatedVfdUsers
    ++ generatedProviderResources;

  generatedByZone = lib.foldl'
    (result: row:
      result // {
        ${row.zoneName} = (result.${row.zoneName} or { }) // {
          ${row.name} = row.resource;
        };
      })
    { }
    generatedRows;

  shorthandRows = lib.mapAttrsToList
    (name: declaration: {
      zoneName = declaration.zone;
      name = name;
      resource = typedResource "Volume" name declaration.zone declaration.spec
        declaration.ownerRef;
    })
    (cfg.volumes or { });

  shorthandByZone = lib.foldl'
    (result: row:
      result // {
        ${row.zoneName} = (result.${row.zoneName} or { }) // {
          ${row.name} = row.resource;
        };
      })
    { }
    shorthandRows;

  generatedNames = lib.concatMap
    (zoneName: map (name: "${zoneName}/${name}")
      (lib.attrNames (generatedByZone.${zoneName} or { })))
    (lib.attrNames generatedByZone);
  authoredNames = lib.concatMap
    (zoneName: map (name: "${zoneName}/${name}")
      (lib.attrNames (cfg.zones.${zoneName}.resources or { })))
    (lib.attrNames cfg.zones);
  shorthandNames = map (row: "${row.zoneName}/${row.name}") shorthandRows;
  collisions = lib.unique (
    lib.filter (name: lib.length (lib.filter (candidate: candidate == name)
      (authoredNames ++ generatedNames ++ shorthandNames)) > 1)
      (authoredNames ++ generatedNames ++ shorthandNames));

  shorthandAssertions = lib.concatMap
    (row: [
      {
        assertion = builtins.hasAttr row.zoneName cfg.zones;
        message = "d2b.volumes.${row.name}.zone must name a declared Zone.";
      }
      {
        assertion = builtins.match resourceNamePattern row.name != null;
        message = "d2b.volumes.${row.name}: name must match ${resourceNamePattern}.";
      }
    ])
    shorthandRows;

  collisionAssertions = [{
    assertion = collisions == [ ];
    message = "Volume compiler resource names collide: ${lib.concatStringsSep ", " collisions}.";
  }];
in
{
  # The unified `d2b.zones.<zone>.resources.<name>.spec` surface remains the
  # canonical authoring form.  This optional shorthand is useful to consumers
  # that want a typed Volume declaration without repeating the surrounding
  # resource envelope; the resource compiler binds it to the same canonical
  # Volume projection and never invents a second JSON schema.
  options.d2b.volumes = lib.mkOption {
    type = types.attrsOf (types.submodule ({ name, ... }: {
      freeformType = null;
      options = {
        zone = lib.mkOption {
          type = types.strMatching "^[a-z][a-z0-9-]{0,62}$";
          description = "Zone receiving this optional Volume declaration.";
        };
        ownerRef = lib.mkOption {
          type = types.nullOr resourceRefType;
          default = null;
          description = "Optional same-Zone owner ResourceRef.";
        };
        spec = lib.mkOption {
          type = volumeSpecType;
          description = "Typed Volume base specification.";
        };
      };
    }));
    default = { };
    description = ''
      Optional typed Volume declarations. The canonical form remains
      d2b.zones.<zone>.resources.<name> with type = "Volume"; this shorthand
      is projected into the same compiler table and does not create a second
      resource schema.
    '';
  };

  config = {
    assertions = shorthandAssertions ++ collisionAssertions;
    d2b._resourceCompiler = {
      volumeOptions = volumeOptionTypes;
      volumeGenerated = {
        byZone = generatedByZone;
        users = generatedVfdUsers;
        providers = generatedProviderResources;
      };
      volumeShorthand = shorthandByZone;
    };
  };
}
