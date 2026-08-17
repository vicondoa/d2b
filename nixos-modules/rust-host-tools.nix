{ inputs, pkgs, lib }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  craneLib = inputs.crane.mkLib pkgs;
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  hostSource = pkgs.runCommand "d2b-host-tools-rust-src" { } ''
    mkdir -p "$out/packages"
    cp -r ${packagesSrc}/. "$out/packages/"
    mkdir -p "$out/docs/reference/schemas/v3/providers"
    cp ${../docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
      "$out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
    cp ${../docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
      "$out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
  '';
  cargoLock = ../packages/Cargo.lock;
  outputHashes = {
    "wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  commonBuildArgs = {
    strictDeps = true;
    cargoExtraArgs = "--locked";
    doCheck = false;
    RUSTC_WRAPPER = "";
    SCCACHE_DIR = "";
  };

  # Keep dependency compilation independent of application source changes.
  cargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-host-tools";
    version = "0.0.0-bootstrap";
    src = craneLib.cleanCargoSource packagesSrc;
    inherit cargoLock outputHashes;
    cargoCheckExtraArgs = "--workspace";
    cargoBuildExtraArgs = "--workspace";
  });

  installBinaries = binaries:
    ''
      mkdir -p "$out/bin"
    ''
    + lib.concatMapStringsSep "\n"
      (binary: ''install -Dm755 "target/release/${binary}" "$out/bin/${binary}"'')
      binaries;

  mkMainPackage =
    { package
    , binaries
    , pname ? package
    }:
    craneLib.buildPackage (commonBuildArgs // {
      inherit pname cargoArtifacts cargoLock outputHashes;
      version = "0.0.0-bootstrap";
      src = hostSource;
      sourceRoot = "d2b-host-tools-rust-src/packages";
      cargoToml = ../packages + "/${package}/Cargo.toml";
      cargoBuildExtraArgs =
        "--package ${package}"
        + lib.concatMapStringsSep "" (binary: " --bin ${binary}") binaries;
      installPhaseCommand = installBinaries binaries;
    });

  broker = craneLib.buildPackage (commonBuildArgs // {
    pname = "d2b-priv-broker";
    version = "0.0.0-bootstrap";
    inherit cargoArtifacts outputHashes;
    src = hostSource;
    sourceRoot = "d2b-host-tools-rust-src/packages/d2b-priv-broker";
    cargoToml = ../packages/d2b-priv-broker/Cargo.toml;
    cargoLock = ../packages/d2b-priv-broker/Cargo.lock;
    cargoBuildExtraArgs = "--no-default-features";
    installPhaseCommand = installBinaries [ "d2b-priv-broker" ];
  });
in
{
  inherit cargoArtifacts broker;

  d2bd = mkMainPackage {
    package = "d2bd";
    binaries = [ "d2bd" ];
  };
  d2b = mkMainPackage {
    package = "d2b";
    binaries = [ "d2b" ];
  };
  activationHelper = mkMainPackage {
    package = "d2b-host";
    binaries = [ "d2b-activation-helper" ];
  };
  hostActivationHelper = mkMainPackage {
    package = "d2b-host-activation-helper";
    binaries = [ "d2b-host-activation-helper" ];
  };
  gatewayRuntime = mkMainPackage {
    package = "d2b-gateway-runtime";
    binaries = [ "d2b-gateway-enroll" "d2b-gateway-relay" ];
  };
  unsafeLocalHelper = mkMainPackage {
    package = "d2b-unsafe-local-helper";
    binaries = [ "d2b-unsafe-local-helper" ];
  };
  resourceCompiler = mkMainPackage {
    package = "d2b-resource-compiler";
    binaries = [ "d2b-resource-compiler" ];
  };
  waylandProxy = mkMainPackage {
    package = "d2b-wayland-proxy";
    binaries = [ "d2b-wayland-proxy" ];
  };
}
