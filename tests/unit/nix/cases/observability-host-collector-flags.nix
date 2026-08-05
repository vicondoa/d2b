# Isolate the host collector enablement failure scenario.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-flags"; })
