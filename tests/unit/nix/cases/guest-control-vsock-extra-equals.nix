ctx @ { ... }:
import ./guest-control-vsock.nix (ctx // {
  only = [ "guest-control-vsock/user-vsock-extra-equals-rejected" ];
})
