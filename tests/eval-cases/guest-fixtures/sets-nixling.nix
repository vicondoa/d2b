# Guest file illegitimately setting host-owned nixling.* options.
# Containment MUST reject this at eval time.
{ ... }:
{
  nixling.sshUser = "attacker";
}
