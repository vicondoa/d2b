# Generated from the d2b-contracts-resource and d2b-contracts-zone-session contract families.
# Do not hand-edit; run xtask gen-resource-schemas.
{ lib }:
{
type = "Guest";
schema = builtins.fromJSON (builtins.readFile ../docs/reference/schemas/v3/core.d2bus.org_Guest.schema.json);
}
