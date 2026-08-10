# Eval-time coverage for the framework-owned Provider ELF shim constructor.
#
# Realization coverage belongs to the derivation itself: the build helper
# resolves both inputs and checks the ELF and fd-relative runtime contract
# before installing the output. These cases pin the public constructor's
# validation boundary and the values it bakes into that derivation, including
# the same-output Python-style interpreter shape and fixed argument ordering.
{ lib, pkgs, flakeRoot, ... }:

let
  buildProviderElfShim =
    import (flakeRoot + "/nix/provider-elf-shim.nix");

  programOutput = pkgs.writeTextFile {
    name = "provider-elf-shim-test-program";
    destination = "/share/d2b/provider/provider-elf-shim-test.py";
    text = "print('provider shim test')\n";
  };

  program = "${programOutput}/share/d2b/provider/provider-elf-shim-test.py";

  base = {
    inherit pkgs program;
    name = "d2b-provider-shim-test";
    interpreterPkg = pkgs.coreutils;
    interpreterPath = "bin/cat";
    extraArgs = [ "-n" ];
  };

  mkShim = overrides:
    buildProviderElfShim (base // overrides);

  evalBuilder = overrides:
    (builtins.tryEval ((mkShim overrides).outPath != "")).success;

  shim = mkShim { };
  metadata = shim.passthru.providerElfShim;
in
{
  "provider-elf-shim/positive-constructor" = {
    expr = builtins.isAttrs shim && shim ? outPath;
    expected = true;
  };

  "provider-elf-shim/output-name-is-validated-entry-name" = {
    expr = metadata.name;
    expected = "d2b-provider-shim-test";
  };

  "provider-elf-shim/interpreter-is-kept-as-output-plus-relative-path" = {
    expr = {
      output = metadata.interpreterOutput;
      relative = metadata.interpreterPath;
      argv0 = metadata.interpreterArgv0;
    };
    expected = {
      output = "${pkgs.coreutils}";
      relative = "bin/cat";
      argv0 = "cat";
    };
  };

  "provider-elf-shim/program-is-a-distinct-store-output" = {
    expr = lib.hasPrefix "/nix/store/" metadata.program
      && metadata.program != metadata.interpreterOutput;
    expected = true;
  };

  "provider-elf-shim/fixed-arguments-are-preserved" = {
    expr = metadata.extraArgs;
    expected = [ "-n" ];
  };

  "provider-elf-shim/python-style-same-output-link-shape-is-accepted" = {
    expr = evalBuilder {
      interpreterPkg = pkgs.python3;
      interpreterPath = "bin/python3";
    };
    expected = true;
  };

  "provider-elf-shim/name-with-uppercase-is-rejected" = {
    expr = evalBuilder { name = "Provider-Shim"; };
    expected = false;
  };

  "provider-elf-shim/name-with-leading-dash-is-rejected" = {
    expr = evalBuilder { name = "-provider-shim"; };
    expected = false;
  };

  "provider-elf-shim/name-over-64-bytes-is-rejected" = {
    expr = evalBuilder {
      name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    };
    expected = false;
  };

  "provider-elf-shim/absolute-interpreter-path-is-rejected" = {
    expr = evalBuilder { interpreterPath = "/bin/cat"; };
    expected = false;
  };

  "provider-elf-shim/parent-interpreter-component-is-rejected" = {
    expr = evalBuilder { interpreterPath = "bin/../cat"; };
    expected = false;
  };

  "provider-elf-shim/empty-interpreter-path-is-rejected" = {
    expr = evalBuilder { interpreterPath = ""; };
    expected = false;
  };

  "provider-elf-shim/empty-interpreter-component-is-rejected" = {
    expr = evalBuilder { interpreterPath = "bin//cat"; };
    expected = false;
  };

  "provider-elf-shim/non-store-interpreter-output-is-rejected" = {
    expr = evalBuilder { interpreterPkg = "/tmp/provider-interpreter"; };
    expected = false;
  };

  "provider-elf-shim/non-store-program-is-rejected" = {
    expr = evalBuilder { program = "/tmp/provider-program"; };
    expected = false;
  };

  "provider-elf-shim/store-root-program-file-is-accepted" = {
    expr = evalBuilder { program = pkgs.writeText "provider-program" "text"; };
    expected = true;
  };

  "provider-elf-shim/non-list-extra-arguments-are-rejected" = {
    expr = evalBuilder { extraArgs = "-n"; };
    expected = false;
  };

  "provider-elf-shim/non-string-extra-argument-is-rejected" = {
    expr = evalBuilder { extraArgs = [ "-n" 1 ]; };
    expected = false;
  };
}
