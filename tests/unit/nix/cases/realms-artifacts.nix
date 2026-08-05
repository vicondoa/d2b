# Isolate realm artifact and host-local controller cases.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "artifacts"; })
