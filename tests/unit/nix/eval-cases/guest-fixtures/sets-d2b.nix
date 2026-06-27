# Guest file illegitimately setting host-owned d2b.* options.
# Containment MUST reject this at eval time.
{ ... }:
{
  d2b.sshUser = "attacker";
}
