# Isolate gateway guest artifact cases from host assertion scenarios.
{ ... }@ctx:
import ./gateway-vm.nix (ctx // { casePartition = "guest"; })
