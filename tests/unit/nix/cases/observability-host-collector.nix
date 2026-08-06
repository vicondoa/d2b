# Host collector observability cases get an independent file job because each
# scenario renders a broad host service graph.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector"; })
