# Isolate realm workload and migration cases from index/artifact scenarios.
{ ... }@ctx:
import ./realms.nix (ctx // { casePartition = "workloads"; })
