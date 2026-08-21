# Prebuilt broker release compatibility and output-shape contract.
#
# The v1.4.1 release asset still contains the legacy broker executable. The
# consumer-facing derivation must keep the released fetch identity while
# exposing the current d2b-broker binary name.
{ lib, pkgs, flakeRoot, ... }:

let
  manifest = builtins.fromJSON (builtins.readFile (flakeRoot + "/nix/prebuilt.json"));
  spec = manifest.binaries."d2b-broker";
  prebuilt = import (flakeRoot + "/nix/prebuilt.nix") { inherit pkgs lib; };
  broker = prebuilt."d2b-broker";
  installPhase = broker.installPhase;
in
{
  "prebuilt-broker/released-asset-identity" = {
    expr = spec.url
      == "https://github.com/vicondoa/d2b/releases/download/v1.4.1/d2b-priv-broker-1.4.1-x86_64-linux.tar.gz"
      && spec.hash == "sha256-qY5eXFpenVG6UrgwwV4nQC+QQxFr3O+mJHVdOSj256A=";
    expected = true;
  };
  "prebuilt-broker/legacy-binary-renamed-to-current-output" = {
    expr = (spec.sourceBinary or null) == "d2b-priv-broker"
      && broker.pname == "d2b-broker"
      && lib.hasInfix "d2b-priv-broker" installPhase
      && lib.hasInfix "$out/bin/d2b-broker" installPhase;
    expected = true;
  };
}
