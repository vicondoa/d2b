# Coverage for the W7fu17 unsafe-local controller-authorization fix:
# nixos-modules/unsafe-local-helper.nix must derive a bounded, sorted, exact
# allowlist of enabled host-local *child* realm controller UIDs from
# `d2b._realmPrincipals.children`, scoped per user to only the realms that
# both host an unsafe-local (systemd-user) workload and name that exact
# user in `allowedUsers`; inject it as a non-secret immutable file; and
# apply it via an exact-uid ACL sync script, all without loosening the base
# 0600/0700 socket/directory mode.
{ mkEval, lib, ... }:

let
  d2bLib = import ../../../../nixos-modules/lib.nix { inherit lib; };
  inherit (d2bLib) stablePrincipalId;

  base = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.groups.desktop = { };
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
      group = "desktop";
    };
    users.users.carol = {
      isNormalUser = true;
      uid = 1001;
      group = "desktop";
    };
    d2b.site.waylandUser = "alice";
  };

  evalWith = overrides: mkEval ([ base ] ++ overrides);

  # Two host-local child realms both host an unsafe-local (systemd-user)
  # workload: "host" (alice only) and "guest" (carol only). A third,
  # "guarded", is allowed for alice too but binds a non-systemd-user
  # runtime, so it must never contribute a controller UID to anyone.
  fixture = evalWith [
    ({ ... }: {
      d2b.realms.host = {
        allowedUsers = [ "alice" ];
        policy.allowUnsafeLocal = true;
        providers.runtime = {
          type = "runtime";
          implementationId = "systemd-user";
        };
        workloads.tools.providerRefs.runtime = "runtime";
      };
      d2b.realms.guest = {
        allowedUsers = [ "carol" ];
        policy.allowUnsafeLocal = true;
        providers.runtime = {
          type = "runtime";
          implementationId = "systemd-user";
        };
        workloads.tools.providerRefs.runtime = "runtime";
      };
      d2b.realms.guarded = {
        allowedUsers = [ "alice" ];
        policy.allowUnsafeLocal = true;
        providers.runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        workloads.tools.providerRefs.runtime = "runtime";
      };
    })
  ];

  identity = import ../../../../nixos-modules/v2-identity.nix;

  cfg = fixture.config;
  authz = cfg.d2b._unsafeLocalRuntimeAuthz;
  allowlist = authz.allowlist;
  aclScriptText = authz.aclScriptText;

  entryFor = user: lib.findFirst (entry: entry.user == user) null allowlist.entries;
  aliceEntry = entryFor "alice";
  carolEntry = entryFor "carol";

  # Resolve each expected controller uid from the realm's own canonical
  # path/id derivation (the same one nixos-modules/realm-users.nix and
  # nixos-modules/index-realms.nix use), rather than by searching
  # `_realmPrincipals.children` by `allowedUsers` -- alice is deliberately
  # also `allowedUsers`-listed on the non-systemd-user "guarded" realm, so a
  # naive first-match-by-user search would silently resolve to the wrong
  # realm's controller.
  hostRealmId = identity.deriveRealmId "host.local-root";
  guestRealmId = identity.deriveRealmId "guest.local-root";
  hostUid = stablePrincipalId "d2bd-r-${hostRealmId}";
  guestUid = stablePrincipalId "d2bd-r-${guestRealmId}";

  socket = cfg.systemd.user.sockets.d2b-runtime-systemd-user;
  service = cfg.systemd.user.services.d2b-runtime-systemd-user;

  # The service's own Environment= is the sole non-secret pointer into the
  # store; the ACL script must reference that exact same generated file
  # (never a second, independently-generated copy).
  allowlistFilePath = builtins.unsafeDiscardStringContext
    (lib.removePrefix "D2B_UNSAFE_LOCAL_CONTROLLER_ALLOWLIST=" service.serviceConfig.Environment);

  # Malformed-allowlist-document rejection is exercised at the Rust layer
  # (packages/d2b-unsafe-local-helper/src/controller_allowlist.rs and
  # src/server.rs), since the document's *shape* (bounded, sorted,
  # deduplicated, non-zero) is a property of the Nix-generated content this
  # module produces, not of a hand-crafted string the Nix side ever parses.
  # Here we instead assert the Nix-generated document itself is already
  # exactly that shape, so it can never be rejected by the Rust parser.
  allEntryUids = lib.concatMap (entry: entry.controllerUids) allowlist.entries;
  allEntryUidsSorted = allEntryUids == lib.sort lib.lessThan allEntryUids;
  allEntryUidsUnique = allEntryUids == lib.unique allEntryUids;
  allEntryUsersUnique =
    let users = map (entry: entry.user) allowlist.entries;
    in users == lib.unique users;
  noZeroUid = !(lib.elem 0 allEntryUids);
in
{
  "unsafe-local/controller-allowlist-grants-exact-controller-to-allowed-user" = {
    expr = {
      inherit (allowlist) schemaVersion;
      aliceControllerUids = aliceEntry.controllerUids;
      carolControllerUids = carolEntry.controllerUids;
    };
    expected = {
      schemaVersion = 1;
      aliceControllerUids = [ hostUid ];
      carolControllerUids = [ guestUid ];
    };
  };

  "unsafe-local/controller-allowlist-excludes-unrelated-controller-and-user" = {
    expr = {
      # alice is never granted guest's controller, and carol is never
      # granted host's controller: cross-user grants must not leak.
      aliceExcludesGuestController = !(lib.elem guestUid aliceEntry.controllerUids);
      carolExcludesHostController = !(lib.elem hostUid carolEntry.controllerUids);
      # only the two systemd-user-backed realms' users get an entry at
      # all; "guarded" (cloud-hypervisor, not unsafe-local) never adds one,
      # even though alice is in its allowedUsers.
      onlyAliceAndCarolHaveEntries =
        lib.sort lib.lessThan (map (entry: entry.user) allowlist.entries)
        == [ "alice" "carol" ];
      # bob was never named in any realm's allowedUsers, so he is not an
      # eligible user at all -- there is no entry, not even an empty one.
      bobHasNoEntry = entryFor "bob" == null;
    };
    expected = {
      aliceExcludesGuestController = true;
      carolExcludesHostController = true;
      onlyAliceAndCarolHaveEntries = true;
      bobHasNoEntry = true;
    };
  };

  "unsafe-local/controller-allowlist-document-is-bounded-sorted-and-deduplicated" = {
    expr = {
      inherit allEntryUidsSorted allEntryUidsUnique allEntryUsersUnique noZeroUid;
      withinEntryCountBound = builtins.length allowlist.entries <= 256;
      withinPerUserUidBound =
        lib.all (entry: builtins.length entry.controllerUids <= 64) allowlist.entries;
    };
    expected = {
      allEntryUidsSorted = true;
      allEntryUidsUnique = true;
      allEntryUsersUnique = true;
      noZeroUid = true;
      withinEntryCountBound = true;
      withinPerUserUidBound = true;
    };
  };

  "unsafe-local/acl-sync-script-shape-grants-exact-controller-uid-only" = {
    expr = {
      resetsDirectoryAcl = lib.hasInfix ''setfacl -b "$dir"'' aclScriptText;
      resetsSocketAcl = lib.hasInfix ''setfacl -b "$sock"'' aclScriptText;
      grantsDirectoryTraverse =
        lib.hasInfix ''setfacl -m "u:''${controller_uid}:x" "$dir"'' aclScriptText;
      grantsSocketReadWrite =
        lib.hasInfix ''setfacl -m "u:''${controller_uid}:rw" "$sock"'' aclScriptText;
      # The script never bakes in a fixed uid or username of its own; it
      # always resolves its own identity at runtime and only ever grants
      # the *other*, allowlisted controller uid(s) -- never a broader
      # same-uid, group, or "all d2bd processes" grant.
      resolvesOwnIdentityAtRuntime =
        lib.hasInfix "id -un" aclScriptText && lib.hasInfix "id -u" aclScriptText;
      referencesTheGeneratedAllowlistFile =
        lib.hasInfix allowlistFilePath aclScriptText;
      neverGrantsGroupOrOtherAcl =
        !(lib.hasInfix ''setfacl -m "g:'' aclScriptText)
        && !(lib.hasInfix ''setfacl -m "o:'' aclScriptText);
    };
    expected = {
      resetsDirectoryAcl = true;
      resetsSocketAcl = true;
      grantsDirectoryTraverse = true;
      grantsSocketReadWrite = true;
      resolvesOwnIdentityAtRuntime = true;
      referencesTheGeneratedAllowlistFile = true;
      neverGrantsGroupOrOtherAcl = true;
    };
  };

  "unsafe-local/base-socket-and-directory-mode-and-endpoint-role-are-unchanged" = {
    expr = {
      socketMode = socket.socketConfig.SocketMode;
      directoryMode = socket.socketConfig.DirectoryMode;
      socketPath = socket.socketConfig.ListenSequentialPacket;
      # The daemon-restart/no-cross-uid-execution invariant: the helper's
      # own ExecStart is completely unaffected by the allowlist wiring --
      # it is always the same fixed binary, never selected or parameterized
      # by any peer-supplied or allowlist-derived value.
      execStartUnparameterizedByAllowlist =
        !(lib.hasInfix "ALLOWLIST" service.serviceConfig.ExecStart)
        && !(lib.hasInfix "controller" service.serviceConfig.ExecStart);
      environmentCarriesOnlyTheAllowlistPointer =
        lib.hasPrefix "D2B_UNSAFE_LOCAL_CONTROLLER_ALLOWLIST="
          service.serviceConfig.Environment;
    };
    expected = {
      socketMode = "0600";
      directoryMode = "0700";
      socketPath = "/run/d2b/u/%U/runtime-agent.sock";
      execStartUnparameterizedByAllowlist = true;
      environmentCarriesOnlyTheAllowlistPointer = true;
    };
  };
}
