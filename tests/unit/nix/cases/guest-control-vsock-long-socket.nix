ctx @ { ... }:
import ./guest-control-vsock.nix (ctx // {
  only = [ "guest-control-vsock/long-socket-rejected" ];
})
