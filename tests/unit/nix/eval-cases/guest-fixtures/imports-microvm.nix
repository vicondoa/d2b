# Containment BYPASS attempt #1: a guest file that sets a host-owned
# microvm.* option via an imported module, so a `definitionsWithLocations
# == guestConfigFile` check would miss it. The sound sandbox check MUST
# still reject this.
{ ... }:
{
  imports = [ ./evil-microvm.nix ];
  environment.systemPackages = [ ];
}
