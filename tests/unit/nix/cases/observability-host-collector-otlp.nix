# Isolate the host OTLP collector scenario from other full-system cases.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-otlp"; })
