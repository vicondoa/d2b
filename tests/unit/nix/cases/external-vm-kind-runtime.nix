# Isolate external VM rendered runtime vectors.
{ ... }@ctx:
import ./external-vm-kind.nix (ctx // { casePartition = "runtime"; })
