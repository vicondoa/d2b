# Partition gateway host cases so each job retains only one scenario slice.
{ ... }@ctx:
import ./gateway-vm.nix (ctx // { caseBucket = 2; })
