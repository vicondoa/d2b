{ pkgs, self, system }:

pkgs.runCommand "d2b-guest-runtime-static" { } ''
  mkdir -p "$out/bin"
  cp ${self.packages.${system}.d2bd-guest-static}/bin/d2bd "$out/bin/d2bd"
  cp ${self.packages.${system}.d2b-broker-guest-static}/bin/d2b-broker \
    "$out/bin/d2b-broker"
''
