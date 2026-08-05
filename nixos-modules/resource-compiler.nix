# The single realised Phase 2 compiler used by the v3 resource bundle.
#
# The compiler is built from the workspace rather than imported from a
# consumer's ambient PATH. This keeps the Nix build hermetic and ensures the
# Rust implementation, its Cargo lock, and the bundle derivation move
# together.
{ config, lib, pkgs, ... }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" =
      "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  compilerPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-resource-compiler";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [
      "--package"
      "d2b-resource-compiler"
      "--bin"
      "d2b-resource-compiler"
    ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      printf '%s\n' '[build]' 'rustc-wrapper = ""' > .cargo/config.toml
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-resource-compiler \
        "$out/bin/d2b-resource-compiler" 2>/dev/null \
        || install -Dm755 target/release/d2b-resource-compiler \
          "$out/bin/d2b-resource-compiler"
      runHook postInstall
    '';
  };

  resourceTypes = import ./generated/resource-types.nix;
  schemaRoot = pkgs.linkFarm "d2b-resource-schemas" (map
    (resourceType: {
      name = "core.d2bus.org_${resourceType}.schema.json";
      path = ../docs/reference/schemas/v3
        + "/core.d2bus.org_${resourceType}.schema.json";
    })
    resourceTypes);
in
{
  config.d2b._resourceCompiler.phase2 = {
    compiler = compilerPackage;
    schemaRoot = schemaRoot;
    strictSecrets = true;
  };
}
