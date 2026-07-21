{ config, lib, ... }:

let
  cfg = config.d2b;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);

  mkEndpoint =
    row: endpointKind:
    let
      isPublic = endpointKind == "public";
      principals =
        if isPublic
        then row.socketPrincipals.public
        else row.socketPrincipals.broker;
    in
    {
      endpointId = "realm-${row.realmId}-${endpointKind}-listener";
      inherit endpointKind;
      realmId = row.realmId;
      realmPath = row.realmPath;
      path =
        if isPublic
        then row.resources.publicSocket.path
        else row.resources.brokerSocket.path;
      socketType = "seqpacket";
      bindOwner = "local-root-allocator";
      handoffRole = if isPublic then "controller" else "broker";
      fdName = "${endpointKind}-listener";
      inherit (principals) acceptor owner group mode;
      preBindRequired = true;
      systemdActivation = false;
      selfBind = false;
    };

  rows = lib.concatMap (
    row: [
      (mkEndpoint row "public")
      (mkEndpoint row "broker")
    ]
  ) childRealms;
in
{
  options.d2b._realmEndpointRows = lib.mkOption {
    type = lib.types.listOf lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config.d2b._realmEndpointRows = rows;
}
