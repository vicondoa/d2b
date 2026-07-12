{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  sourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-unsafe-local-helper";
    version = "0.0.0-bootstrap";
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
  helperPackage =
    if prebuilt != null && prebuilt ? "d2b-unsafe-local-helper"
    then prebuilt."d2b-unsafe-local-helper"
    else sourcePackage;
  unsafeLocalRealms = lib.filter
    (realm:
      lib.any
        (workload: workload.enable && workload.kind == "unsafe-local")
        realm.workloads)
    cfg._index.realms.enabledList;
  eligibleUsers = lib.sort lib.lessThan
    (lib.unique (lib.concatMap (realm: realm.allowedUsers) unsafeLocalRealms));
in
{
  config = lib.mkIf cfg.daemonExperimental.enable {
    users.groups.d2b-unsafe-local = { };
    users.users = lib.genAttrs eligibleUsers (_: {
      extraGroups = [ "d2b-unsafe-local" ];
    });

    d2b._hostToolPackages.d2bUnsafeLocalHelper = helperPackage;
    environment.systemPackages = [ helperPackage ];

    systemd.user.services.d2b-unsafe-local-helper = {
      description = "d2b same-uid unsafe-local runtime helper";
      wantedBy = [ "default.target" ];
      unitConfig.ConditionGroup = "d2b-unsafe-local";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${helperPackage}/bin/d2b-unsafe-local-helper --wayland-proxy ${cfg._hostToolPackages.d2bWaylandProxy}/bin/d2b-wayland-proxy";
        Restart = "on-failure";
        RestartSec = "5s";
        Slice = "app.slice";
      };
    };
  };
}
