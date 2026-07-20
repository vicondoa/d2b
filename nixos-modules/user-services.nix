{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  userdPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-userd";
    version = "2.0.0";
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [ "--package" "d2b-userd" ];
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
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-userd \
        $out/bin/d2b-userd 2>/dev/null \
        || install -Dm755 target/release/d2b-userd $out/bin/d2b-userd
      runHook postInstall
    '';
  };
  endpointUsers = lib.sort lib.lessThan (lib.unique (
    cfg.site.adminUsers
    ++ cfg.site.launcherUsers
    ++ lib.concatMap (realm: realm.allowedUsers) cfg._index.realms.enabledList
  ));
  endpointTmpfiles = lib.concatMap (user:
    let
      uid = toString config.users.users.${user}.uid;
      group = config.users.users.${user}.group;
    in [
      "a+ /run/d2b - - - - u:${user}:--x"
      "d /run/d2b/u/${uid} 0700 ${user} ${group} -"
      "z /run/d2b/u/${uid} 0700 ${user} ${group} -"
    ]) endpointUsers;
in
{
  config = lib.mkIf cfg.daemonExperimental.enable {
    users.groups.d2b-user-services = { };
    users.users = lib.genAttrs endpointUsers (_: {
      extraGroups = [ "d2b-user-services" ];
    });

    environment.systemPackages = [ userdPackage ];
    systemd.tmpfiles.rules = [
      "d /run/d2b/u 0711 root root -"
      "z /run/d2b/u 0711 root root -"
    ] ++ endpointTmpfiles;

    systemd.user.sockets.d2b-userd = {
      description = "d2b authenticated user service endpoint";
      wantedBy = [ "sockets.target" ];
      unitConfig.ConditionGroup = "d2b-user-services";
      socketConfig = {
        ListenSequentialPacket = "/run/d2b/u/%U/userd.sock";
        FileDescriptorName = "user-agent";
        SocketMode = "0600";
        DirectoryMode = "0700";
        RemoveOnStop = true;
        Service = "d2b-userd.service";
      };
    };

    systemd.user.services.d2b-userd = {
      description = "d2b authenticated user service";
      requires = [ "d2b-userd.socket" ];
      after = [ "d2b-userd.socket" ];
      unitConfig.ConditionGroup = "d2b-user-services";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${userdPackage}/bin/d2b-userd";
        Restart = "on-failure";
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
