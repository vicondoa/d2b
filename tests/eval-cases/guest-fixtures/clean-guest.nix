# Contained guest-editable OS layer: only guest OS options. Must pass.
{ ... }:
{
  environment.systemPackages = [ ];
  services.openssh.enable = true;
  users.users.alice.shell = "/run/current-system/sw/bin/bash";
}
