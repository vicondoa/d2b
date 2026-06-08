# Guest file illegitimately setting host-owned microvm.* options.
# Containment MUST reject this at eval time.
{ ... }:
{
  environment.systemPackages = [ ];
  microvm.mem = 4096;
  microvm.cloud-hypervisor.extraArgs = [ "--break-out" ];
}
