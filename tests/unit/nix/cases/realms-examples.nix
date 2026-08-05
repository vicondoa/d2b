# Isolate example configuration realm cases.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "examples"; })
