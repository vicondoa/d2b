{ inputs, pkgs, lib }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  craneLib = inputs.crane.mkLib pkgs;
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  cargoLock = ../packages/Cargo.lock;
  # Keep the deps-only derivation keyed to manifests, locks, and Cargo config,
  # not to application source files. Crane still creates the dummy Rust
  # sources, but giving it this smaller input prevents .rs edits from changing
  # the dummy-source derivation itself.
  cargoManifestSrc = lib.cleanSourceWith {
    src = packagesSrc;
    name = "d2b-cargo-manifests";
    filter = path: type:
      type == "directory"
      || lib.hasSuffix "/Cargo.toml" (toString path)
      || lib.hasSuffix "/Cargo.lock" (toString path)
      || lib.hasSuffix "/.cargo/config.toml" (toString path);
  };
  dummyPackages = craneLib.mkDummySrc {
    src = cargoManifestSrc;
    inherit cargoLock;
  };
  dummySource = pkgs.runCommand "d2b-provider-rust-src" { } ''
    mkdir -p "$out/packages"
    cp -r ${dummyPackages}/. "$out/packages/"
    chmod -R u+w "$out/packages"
    cp ${../packages/Cargo.toml} "$out/packages/Cargo.toml"
  '';

  cratePathName = rel:
    let
      trimmed = lib.removeSuffix "/" (lib.removePrefix "./" rel);
      name = lib.removePrefix "../" trimmed;
    in
    if name == trimmed && lib.hasPrefix "/" trimmed
    then null
    else name;

  pathDepNames = cargoToml:
    let
      parsed = builtins.fromTOML (builtins.readFile cargoToml);
      collect = attrs:
        lib.filter (value: builtins.isAttrs value && value ? path)
          (lib.attrValues attrs);
      names = map (value: cratePathName value.path) (
        collect (parsed.dependencies or { })
        ++ collect (parsed.build-dependencies or { })
      );
    in
    lib.filter (name: name != null && name != "") names;

  crateClosure = package:
    let
      go = seen: queue:
        if queue == [ ] then seen
        else
          let
            name = builtins.head queue;
            rest = builtins.tail queue;
          in
          if builtins.elem name seen then go seen rest
          else
            let
              toml = ../packages + "/${name}/Cargo.toml";
              deps = if builtins.pathExists toml then pathDepNames toml else [ ];
            in
            go (seen ++ [ name ]) (rest ++ deps);
    in
    go [ ] [ package ];

  crateSource = name:
    d2bLib.cleanRustPackagesSource (../packages + "/${name}");

  # Overlay real sources for one package and its path-dep closure onto the
  # dummy workspace. Each crate is a separate Nix path input so a d2bd edit
  # does not rebuild wayland-proxy, the resource compiler, or other host tools.
  mkPackageSource = package:
    let
      crates = lib.filter
        (name: builtins.pathExists (../packages + "/${name}"))
        (crateClosure package);
    in
    pkgs.runCommand "d2b-provider-rust-src-${package}" { } ''
      mkdir -p "$out/packages"
      cp -a ${dummySource}/packages/. "$out/packages/"
      chmod -R u+w "$out/packages"
      ${lib.concatMapStringsSep "\n" (name: ''
        rm -rf "$out/packages/${name}"
        cp -a ${crateSource name} "$out/packages/${name}"
      '') crates}
      mkdir -p "$out/docs/reference/schemas/v3/providers"
      cp ${../docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
        "$out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
      cp ${../docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
        "$out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
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
  hostPackages = [
    "d2bd"
    "d2b"
    "d2b-host"
    "d2b-host-activation-helper"
    "d2b-gateway-runtime"
    "d2b-unsafe-local-helper"
    "d2b-resource-compiler"
    "d2b-wayland-proxy"
  ];
  hostPackageArgs = lib.concatMapStringsSep " " (package: "--package ${package}") hostPackages;

  rustcWrapper = pkgs.writeShellScript "d2b-sccache-rustc-wrapper" ''
    if [ -n "''${SCCACHE_DIR:-}" ] \
      && [ -d "''${SCCACHE_DIR}" ] \
      && [ -w "''${SCCACHE_DIR}" ] \
      && command -v sccache >/dev/null 2>&1; then
      exec sccache "$@"
    fi
    exec "$@"
  '';

  # The wrapper keeps CI and ordinary sandbox builds hermetic when no cache
  # bind mount is available, while host-integration builds can opt into the
  # persistent cache by passing SCCACHE_DIR and extra-sandbox-paths. The
  # host lane evaluates with --impure so this path becomes a fixed derivation
  # environment value; pure CI evaluation leaves it empty and falls back.
  sccacheDir = builtins.getEnv "SCCACHE_DIR";
  commonBuildArgs = {
    strictDeps = true;
    cargoExtraArgs = "--locked";
    inherit cargoVendorDir;
    doCheck = false;
    nativeBuildInputs = [ pkgs.protobuf pkgs.sccache ];
    RUSTC_WRAPPER = rustcWrapper;
    SCCACHE_DIR = sccacheDir;
  };

  cargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-host-tools";
    version = "0.0.0-bootstrap";
    dummySrc = dummySource;
    sourceRoot = "d2b-provider-rust-src/packages";
    cargoToml = ../packages/Cargo.toml;
    inherit cargoLock outputHashes;
    cargoCheckExtraArgs = hostPackageArgs;
    cargoBuildExtraArgs = hostPackageArgs;
  });

  brokerCargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-priv-broker";
    version = "0.0.0-bootstrap";
    dummySrc = dummySource;
    sourceRoot = "d2b-provider-rust-src/packages/d2b-priv-broker";
    cargoToml = ../packages/d2b-priv-broker/Cargo.toml;
    cargoLock = brokerCargoLock;
    cargoVendorDir = brokerCargoVendorDir;
    inherit outputHashes;
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
      src = mkPackageSource package;
      sourceRoot = "d2b-provider-rust-src-${package}/packages";
      cargoToml = ../packages + "/${package}/Cargo.toml";
      cargoBuildExtraArgs =
        "--package ${package}"
        + lib.concatMapStringsSep "" (binary: " --bin ${binary}") binaries;
      installPhaseCommand = installBinaries binaries;
    });

  broker = craneLib.buildPackage (commonBuildArgs // {
    pname = "d2b-priv-broker";
    version = "0.0.0-bootstrap";
    cargoArtifacts = brokerCargoArtifacts;
    inherit outputHashes;
    src = mkPackageSource "d2b-priv-broker";
    sourceRoot = "d2b-provider-rust-src-d2b-priv-broker/packages/d2b-priv-broker";
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
