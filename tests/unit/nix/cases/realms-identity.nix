# Isolate realm identity artifact and secret rejection cases.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "identity"; })
