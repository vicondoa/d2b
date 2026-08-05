# Isolate Zone-control resource compiler cases from legacy realm scenarios.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "zone-control"; })
