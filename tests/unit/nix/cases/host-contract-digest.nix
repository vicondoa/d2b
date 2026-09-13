{ mkModuleEval, lib, ... }:

let
  # A self-contained authoring surface: one Zone with a Role, a RoleBinding
  # bound to a host user, and an unrelated resource the contract must not
  # aggregate.
  resources = {
    operator-reader = {
      type = "Role";
      spec = {
        providerRef = "Provider/system-core";
        rules = [ ];
      };
    };
    operator-reader-binding = {
      type = "RoleBinding";
      metadata.ownerRef = "Provider/system-core";
      spec = {
        roleRef = "Role/operator-reader";
        subjects = [ "User/alice" ];
      };
    };
    audit-archive = {
      type = "Volume";
      spec = { providerRef = "Provider/volume-local"; };
    };
  };
  evaluated = (mkModuleEval [
    {
      d2b.zones.local-root.resources = resources;
    }
  ]).config;
  contract = evaluated.d2b._hostContract;
in
{
  # The golden digest is the contract's identity: it covers the operator
  # bindings, the committed principal allocation, and the declared storage
  # roots, so a row that moves without regenerating the document fails here.
  "host-contract/digest-covers-the-aggregated-rows" = {
    expr = {
      prefixed = lib.hasPrefix "sha256:" contract.digest
        && builtins.stringLength contract.digest == 71;
      golden = contract.digest
        == "sha256:f6c138da465614a39091a6eabb41d8a3b86782a85ff51e14e46e1f10b60b6cc0";
      stable = contract.preimageJson
        != null
        && builtins.length contract.bindings == 1
        && (builtins.head contract.bindings).name == "operator-reader-binding"
        && (builtins.head contract.bindings).owner == "Provider/system-core"
        && builtins.length contract.principals >= 1
        && builtins.length contract.storageRoots >= 1;
    };
    expected = {
      prefixed = true;
      golden = true;
      stable = true;
    };
  };
}
