# Generated from packages/d2b-contracts/src/v3/endpoint.rs.
# Do not hand-edit; run xtask gen-resource-schemas.
{ lib }:
{
type = "Endpoint";
schema = builtins.fromJSON (builtins.readFile ../docs/reference/schemas/v3/core.d2bus.org_Endpoint.schema.json);
}
