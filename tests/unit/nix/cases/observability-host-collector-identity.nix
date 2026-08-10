# Isolate the host identity scenario from other full-system cases.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-identity"; })
