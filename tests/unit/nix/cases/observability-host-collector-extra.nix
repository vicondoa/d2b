# Remaining host collector observability cases get an independent job from
# the primary collector cases to bound retained full-system scenarios.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "host-collector-journal"; })
