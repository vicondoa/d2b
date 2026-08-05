# Guest-heavy observability cases are evaluated in a separate file job so the
# host collector and manifest cases cannot retain their full VM graphs.
{ ... }@ctx:
import ./observability.nix (ctx // { casePartition = "guest"; })
