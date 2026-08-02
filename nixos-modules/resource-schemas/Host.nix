# Generated from packages/d2b-contracts/src/v3/host.rs.
# Do not hand-edit; run xtask gen-resource-schemas.
{ lib }:
{
type = "Host";
schema = builtins.fromJSON (builtins.readFile ../docs/reference/schemas/v3/Host.schema.json);
}
