# Isolate realm rejection vectors from positive wiring scenarios.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "rejections"; })
