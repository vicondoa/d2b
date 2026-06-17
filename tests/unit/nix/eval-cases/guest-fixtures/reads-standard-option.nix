# Contained guest config that READS a standard NixOS option it did not
# itself set (a common `mkIf` guard pattern). The containment check must
# evaluate this WITHOUT crashing/false-positiving — it touches only
# guest OS options. Regression for the freeform-sandbox false positive.
{ config, lib, ... }:
{
  environment.systemPackages = [ ];
  services.openssh.enable = lib.mkIf (config.networking.hostName == "corp-vm") true;
}
