ctx @ { ... }:
import ./guest-control-vsock.nix (ctx // {
  only = [ "guest-control-vsock/user-vsock-socket-rejected" ];
})
