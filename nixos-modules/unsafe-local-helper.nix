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
in
{
  config = lib.mkIf cfg.daemonExperimental.enable {
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
  };
}
