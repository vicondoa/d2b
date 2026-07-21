{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  sourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-unsafe-local-helper";
    version = "2.0.0";
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [ "--package" "d2b-unsafe-local-helper" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-unsafe-local-helper \
        $out/bin/d2b-unsafe-local-helper 2>/dev/null \
        || install -Dm755 target/release/d2b-unsafe-local-helper \
          $out/bin/d2b-unsafe-local-helper
      runHook postInstall
    '';
  };
  helperPackage = sourcePackage;
  # Canonical authority: a workload is unsafe-local iff its normalized
  # runtime provider binding resolves to the `systemd-user` implementation.
  # There is no `spec.kind` field in the closed workload schema (see
  # nixos-modules/options-realms-workloads.nix); selecting on it is
  # unreachable dead code that silently excludes every real unsafe-local
  # workload from this group. Fail-closed here means: no normalized
  # systemd-user binding => no group membership, matching the pattern
  # already used by nixos-modules/{bundle-artifacts,clipboard}.nix.
  isUnsafeLocalWorkload = workload:
    let runtime = workload.providerBindings.runtime or null;
    in runtime != null && runtime.implementationId == "systemd-user";
  unsafeLocalWorkloads =
    lib.filter isUnsafeLocalWorkload cfg._index.workloads.enabledList;
  unsafeLocalRealmIds =
    lib.unique (map (workload: workload.realmId) unsafeLocalWorkloads);
  unsafeLocalRealms =
    map (realmId: cfg._index.realms.enabledById.${realmId}) unsafeLocalRealmIds;
  eligibleUsers = lib.sort lib.lessThan
    (lib.unique (lib.concatMap
      (realm: cfg.realms.${realm.realmName}.allowedUsers)
      unsafeLocalRealms));

  # Narrow authorization surface: only the fixed, Nix-derived controller
  # identities of *enabled host-local child realms that actually host an
  # unsafe-local workload* may ever reach a helper user's endpoint from a
  # different uid. `_realmPrincipals.localRoot` (the `d2bd` controller) is
  # deliberately excluded: unlike child-realm controllers it has no
  # `stablePrincipalId`-derived fixed UID (it's a NixOS-allocated dynamic
  # system-user uid), so it cannot be resolved into this static allowlist at
  # eval time, and it has no `allowedUsers` field to scope against anyway.
  relevantPrincipalRows = lib.filter
    (row: builtins.elem row.realmId unsafeLocalRealmIds)
    cfg._realmPrincipals.children;

  # For one helper user: the bounded, sorted, deduplicated set of controller
  # UIDs whose realm explicitly names that exact user in `allowedUsers`.
  controllerUidsForUser = user:
    lib.sort lib.lessThan (lib.unique (map (row: d2bLib.stablePrincipalId row.controller)
      (lib.filter (row: builtins.elem user row.allowedUsers) relevantPrincipalRows)));

  # Immutable, Nix-owned, non-secret document: which controller UIDs may
  # reach which helper user. Every eligible user gets a row (possibly with an
  # empty list) so the file is a complete, auditable statement of intent
  # rather than an list of exceptions.
  controllerAllowlistData = {
    schemaVersion = 1;
    entries = map (user: {
      inherit user;
      controllerUids = controllerUidsForUser user;
    }) eligibleUsers;
  };
  controllerAllowlistFile = pkgs.writeText "d2b-unsafe-local-controller-allowlist.json"
    (builtins.toJSON controllerAllowlistData);

  # Applies the exact per-user controller grants as POSIX ACLs on the
  # per-user runtime directory and socket. Kept as a plain string (not yet
  # wrapped in a derivation) so it can also be asserted on, string-level, in
  # pure Nix-unit evaluation without IFD.
  aclSyncScriptText = ''
    set -euo pipefail
    user="$(${pkgs.coreutils}/bin/id -un)"
    uid="$(${pkgs.coreutils}/bin/id -u)"
    dir="/run/d2b/u/''${uid}"
    sock="''${dir}/runtime-agent.sock"

    # Reset to exactly the base ACL first so a shrunk allowlist (e.g. after
    # a realm is disabled) never leaves a stale grant behind, and so this
    # script is idempotent across repeated socket activations.
    ${pkgs.acl}/bin/setfacl -b "$dir"
    ${pkgs.acl}/bin/setfacl -b "$sock"

    controller_uids="$(${pkgs.jq}/bin/jq -r --arg user "$user" \
      '(.entries[]? | select(.user == $user) | .controllerUids[]?)' \
      ${controllerAllowlistFile})"

    if [ -n "$controller_uids" ]; then
      while IFS= read -r controller_uid; do
        [ -n "$controller_uid" ] || continue
        ${pkgs.acl}/bin/setfacl -m "u:''${controller_uid}:x" "$dir"
        ${pkgs.acl}/bin/setfacl -m "u:''${controller_uid}:rw" "$sock"
      done <<< "$controller_uids"
    fi
  '';
  aclSyncScript = pkgs.writeShellScript "d2b-unsafe-local-acl-sync" aclSyncScriptText;
in
{
  # Internal, testable projection of the derived controller allowlist and
  # ACL-sync logic. Set unconditionally (not gated by
  # `daemonExperimental.enable`) so nix-unit fixtures can assert on it
  # without needing the full feature flag enabled, matching the
  # `_realmPrincipals` precedent in nixos-modules/realm-users.nix.
  options.d2b._unsafeLocalRuntimeAuthz = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = lib.mkMerge [
    {
      d2b._unsafeLocalRuntimeAuthz = {
        allowlist = controllerAllowlistData;
        aclScriptText = aclSyncScriptText;
      };
    }
    (lib.mkIf cfg.daemonExperimental.enable {
      users.groups.d2b-unsafe-local = { };
      users.users = lib.genAttrs eligibleUsers (_: {
        extraGroups = [ "d2b-unsafe-local" ];
      });

      d2b._hostToolPackages.d2bUnsafeLocalHelper = helperPackage;
      environment.systemPackages = [ helperPackage ];

      systemd.user.sockets.d2b-runtime-systemd-user = {
        description = "d2b authenticated systemd user runtime endpoint";
        wantedBy = [ "sockets.target" ];
        unitConfig.ConditionGroup = "d2b-unsafe-local";
        socketConfig = {
          ListenSequentialPacket = "/run/d2b/u/%U/runtime-agent.sock";
          FileDescriptorName = "runtime-systemd-user";
          SocketMode = "0600";
          DirectoryMode = "0700";
          RemoveOnStop = true;
          Service = "d2b-runtime-systemd-user.service";
          # Grants the exact, bounded set of realm controller UIDs
          # authorized for this user traversal(x)/connect(rw) ACLs on the
          # per-user directory and socket, on top of the unchanged 0700/0600
          # base mode. Runs once the socket exists (both the directory and
          # the socket special file are already present by ExecStartPost
          # time), and resets to the exact base ACL first so grants can
          # never accumulate stale entries across activations.
          ExecStartPost = "${aclSyncScript}";
        };
      };

      systemd.user.services.d2b-runtime-systemd-user = {
        description = "d2b authenticated same-uid systemd user runtime";
        requires = [ "d2b-runtime-systemd-user.socket" ];
        after = [ "d2b-runtime-systemd-user.socket" ];
        unitConfig.ConditionGroup = "d2b-unsafe-local";
        serviceConfig = {
          Type = "simple";
          ExecStart = "${helperPackage}/bin/d2b-unsafe-local-helper";
          # Non-secret, immutable, Nix-owned pointer to this user's exact
          # controller allowlist document (see controller_allowlist.rs).
          # Absent/unset falls back to the safe same-uid-only default.
          Environment = "D2B_UNSAFE_LOCAL_CONTROLLER_ALLOWLIST=${controllerAllowlistFile}";
          Restart = "on-failure";
          RestartPreventExitStatus = "78";
          RestartSec = "5s";
          Slice = "app.slice";
          UMask = "0077";
          NoNewPrivileges = true;
          LockPersonality = true;
          MemoryDenyWriteExecute = true;
          RestrictRealtime = true;
          RestrictSUIDSGID = true;
        };
      };
    })
  ];
}
