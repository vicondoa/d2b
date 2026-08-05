# Isolate the host collector umask scenario from other full-system cases.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-umask"; })
