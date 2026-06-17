# Containment BYPASS attempt #2: a guest file that spoofs its module
# `_file` so its forbidden microvm.* definition is attributed to a
# different path. The sound sandbox check MUST still reject this.
{ ... }:
{
  _file = "/spoofed-not-the-guest-file.nix";
  microvm.mem = 4096;
}
