# Generated from packages/d2b-contracts/src/v3/ephemeral_process.rs.
# Do not hand-edit; run xtask gen-resource-schemas.
{ lib }:
{
type = "EphemeralProcess";
schema = builtins.fromJSON (builtins.readFile ../docs/reference/schemas/v3/core.d2bus.org_EphemeralProcess.schema.json);
}
