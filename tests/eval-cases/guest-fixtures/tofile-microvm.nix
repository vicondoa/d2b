# Containment BYPASS attempt #3: a guest file that sets a host-owned
# microvm.* option through a module GENERATED at eval time via
# `builtins.toFile` (so there is no on-disk source path to attribute the
# definition to). Detection by definition-existence MUST still reject
# this.
{ ... }:
{
  imports = [
    (import (builtins.toFile "generated-microvm.nix" ''
      { ... }: { microvm.mem = 4096; }
    ''))
  ];
  environment.systemPackages = [ ];
}
