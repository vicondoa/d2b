# Partition the assertion corpus so one evaluator does not retain all cases.
{ ... }@ctx:
import ./assertions.nix (ctx // { caseBucket = 1; })
