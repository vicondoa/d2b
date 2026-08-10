# Isolate external VM validation rejection vectors.
{ ... }@ctx:
import ./external-vm-kind.nix (ctx // { casePartition = "rejections"; })
