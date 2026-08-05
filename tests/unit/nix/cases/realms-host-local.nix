# Isolate host-local realm unit cases.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "host-local"; })
