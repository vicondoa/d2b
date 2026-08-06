# Isolate realm controller artifact cases.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "controller"; })
