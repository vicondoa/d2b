# Isolate the host processor split scenario from other full-system cases.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-processor-split"; })
