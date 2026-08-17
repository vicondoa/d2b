{ inputs, pkgs, lib }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  craneLib = inputs.crane.mkLib pkgs;
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  hostSource = pkgs.runCommand "d2b-provider-rust-src" { } ''
    mkdir -p "$out/packages"
    cp -r ${packagesSrc}/. "$out/packages/"
    mkdir -p "$out/docs/reference/schemas/v3/providers"
    cp ${../docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
      "$out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
    cp ${../docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
      "$out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
  '';
  cargoLock = ../packages/Cargo.lock;
  dummySource = pkgs.runCommand "d2b-provider-rust-src" { } ''
    mkdir -p "$out/packages"
    cp -r ${craneLib.mkDummySrc {
      src = packagesSrc;
      inherit cargoLock;
    }}/. "$out/packages/"
  '';
  outputHashes = {
    "git+https://github.com/vicondoa/wl-proxy.git?rev=072945b59fef21a2a8166460454280d543f48772#072945b59fef21a2a8166460454280d543f48772" =
      "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  cargoVendorDir = craneLib.vendorCargoDeps {
    inherit cargoLock outputHashes;
  };
  brokerCargoLock = ../packages/d2b-priv-broker/Cargo.lock;
  brokerCargoVendorDir = craneLib.vendorCargoDeps {
    cargoLock = brokerCargoLock;
    inherit outputHashes;
  };
  commonBuildArgs = {
    strictDeps = true;
    cargoExtraArgs = "--locked";
    inherit cargoVendorDir;
    doCheck = false;
    nativeBuildInputs = [ pkgs.protobuf ];
    RUSTC_WRAPPER = "";
    SCCACHE_DIR = "";
  };

  # Keep dependency compilation independent of application source changes.
  cargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-host-tools";
    version = "0.0.0-bootstrap";
    dummySrc = dummySource;
    sourceRoot = "d2b-provider-rust-src/packages";
    cargoToml = ../packages/Cargo.toml;
    inherit cargoLock outputHashes;
    cargoCheckExtraArgs = "--package d2bd";
    cargoBuildExtraArgs = "--package d2bd";
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
      sourceRoot = "d2b-provider-rust-src/packages";
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
    sourceRoot = "d2b-provider-rust-src/packages/d2b-priv-broker";
    cargoToml = ../packages/d2b-priv-broker/Cargo.toml;
    cargoLock = brokerCargoLock;
    cargoVendorDir = brokerCargoVendorDir;
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
