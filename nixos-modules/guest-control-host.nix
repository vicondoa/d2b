# Host-side Guest ComponentSession policy.
#
# Enrollment material is supplied by the target/provider deployment. This
# module deliberately does not materialize the retired guest-control token or
# expose a host path through a Guest virtiofs share.
{ config, lib, ... }:

let
  cfg = config.d2b;
in
{
  assertions = lib.mapAttrsToList (name: vm: {
    assertion = vm.guest.control.auth.tokenFile == null;
    message = ''
      d2b.vms.${name}.guest.control.auth.tokenFile is retired. Guest
      enrollment uses the ComponentSession key contract; remove the tokenFile
      assignment instead of copying a guest-control token into the Guest.
    '';
  }) cfg.vms;
}
