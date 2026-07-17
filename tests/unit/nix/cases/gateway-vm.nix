{ flakeRoot, lib, ... }:

let
  gatewayPath = flakeRoot + "/nixos-modules/gateway-vm.nix";
  networkSource = builtins.readFile (flakeRoot + "/nixos-modules/network.nix");
  policySource = builtins.readFile
    (flakeRoot
      + "/packages/d2b-contract-tests/tests/policy_host_realm_relay.rs");
in
{
  "gateway-vm/legacy-synthesizer-deleted" = {
    expr = builtins.pathExists gatewayPath;
    expected = false;
  };

  "gateway-vm/network-emitter-does-not-synthesize-gateways" = {
    expr =
      !lib.hasInfix "cfg.gateways" networkSource
      && !lib.hasInfix "gatewayVm" networkSource;
    expected = true;
  };

  "gateway-vm/credential-policy-has-no-deleted-file-allowance" = {
    expr = lib.hasInfix ''"nixos-modules/gateway-vm.nix"'' policySource;
    expected = false;
  };
}
