{ inputs, pkgs, lib }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  craneLib = inputs.crane.mkLib pkgs;
  packagesSrc = d2bLib.cleanRustPackagesSource ../.;
  hostSource = pkgs.runCommand "d2b-provider-rust-src" { } ''
    mkdir -p "$out"
    cp -r ${packagesSrc}/. "$out/"
  '';
  cargoLock = ../Cargo.lock;
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
    mkdir -p "$out"
    cp -r ${craneLib.mkDummySrc {
      src = cargoManifestSrc;
      inherit cargoLock;
    }}/. "$out/"
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
  brokerCargoLock = ../Cargo.lock;
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
    "d2b-provider-display-wayland"
  ];
  hostPackageArgs = lib.concatMapStringsSep " " (package: "--package ${package}") hostPackages;
  sccacheCacheSize = "10G";

  rustcWrapper = pkgs.writeShellScript "d2b-sccache-rustc-wrapper" ''
    # The fixed path is absent from ordinary Nix sandboxes, so those builds
    # retain the plain-rustc fallback. Once the opt-in host preflight has
    # exposed the path, any posture or tool failure is an actionable error
    # instead of silently disabling the cache.
    if [ -n "''${SCCACHE_DIR:-}" ] && [ -e "''${SCCACHE_DIR}" ]; then
      if [ ! -d "''${SCCACHE_DIR}" ] || [ ! -w "''${SCCACHE_DIR}" ]; then
        echo "d2b sccache: configured cache ''${SCCACHE_DIR} is not a writable directory" >&2
        exit 1
      fi
      if ! command -v sccache >/dev/null 2>&1; then
        echo "d2b sccache: configured cache requires sccache on PATH" >&2
        exit 1
      fi
      export SCCACHE_CACHE_SIZE="${sccacheCacheSize}"
      exec sccache "$@"
    fi
    exec "$@"
  '';

  # Constant sandbox path for Nix source builds that use these host-tool
  # derivations. The directory is absent in the default sandbox, so the
  # wrapper falls back to rustc. A host that enables the d2b site cache option
  # exposes this fixed path through the Nix daemon's global sandbox settings.
  sccacheDir = "/var/cache/d2b-sccache";
  commonBuildArgs = {
    strictDeps = true;
    cargoExtraArgs = "--locked";
    inherit cargoVendorDir;
    doCheck = false;
    nativeBuildInputs = [ pkgs.protobuf pkgs.sccache ];
    RUSTC_WRAPPER = rustcWrapper;
    SCCACHE_DIR = sccacheDir;
    SCCACHE_CACHE_SIZE = sccacheCacheSize;
  };

  cargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-host-tools";
    version = "0.0.0-bootstrap";
    dummySrc = dummySource;
    sourceRoot = "d2b-provider-rust-src";
    cargoToml = ../Cargo.toml;
    inherit cargoLock outputHashes;
    cargoCheckExtraArgs = hostPackageArgs;
    cargoBuildExtraArgs = hostPackageArgs;
  });

  brokerCargoArtifacts = craneLib.buildDepsOnly (commonBuildArgs // {
    pname = "d2b-broker";
    version = "0.0.0-bootstrap";
    dummySrc = dummySource;
    sourceRoot = "d2b-provider-rust-src";
    cargoToml = ../Cargo.toml;
    cargoLock = brokerCargoLock;
    cargoVendorDir = brokerCargoVendorDir;
    inherit outputHashes;
    cargoCheckExtraArgs = "--package d2b-broker --no-default-features";
    cargoBuildExtraArgs = "--package d2b-broker --no-default-features";
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
      sourceRoot = "d2b-provider-rust-src";
      cargoToml = ../packages + "/${package}/Cargo.toml";
      cargoBuildExtraArgs =
        "--package ${package}"
        + lib.concatMapStringsSep "" (binary: " --bin ${binary}") binaries;
      installPhaseCommand = installBinaries binaries;
    });

  broker = craneLib.buildPackage (commonBuildArgs // {
    pname = "d2b-broker";
    version = "0.0.0-bootstrap";
    cargoArtifacts = brokerCargoArtifacts;
    inherit outputHashes;
    src = hostSource;
    sourceRoot = "d2b-provider-rust-src";
    cargoToml = ../Cargo.toml;
    cargoLock = brokerCargoLock;
    cargoVendorDir = brokerCargoVendorDir;
    cargoBuildExtraArgs = "--no-default-features";
    installPhaseCommand = installBinaries [ "d2b-broker" ];
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
    package = "d2b-provider-display-wayland";
    binaries = [ "d2b-wayland-proxy" ];
  };
}
