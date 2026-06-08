# Helper module imported by imports-microvm.nix to set a host-owned
# option from OUTSIDE the top-level guestConfigFile (the import bypass).
{ ... }:
{
  microvm.mem = 4096;
}
