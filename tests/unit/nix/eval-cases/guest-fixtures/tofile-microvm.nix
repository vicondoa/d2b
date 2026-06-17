# Containment BYPASS attempt #3: a guest file that sets a host-owned
# microvm.* option through a module CONSTRUCTED at eval time (an inline
# module value in `imports`, not imported from any checked-in source file),
# so there is no distinct on-disk source path to attribute the definition
# to. Detection by definition-existence MUST still reject this.
#
# This previously generated the module via `import (builtins.toFile
# "generated-microvm.nix" "...")`. That re-materializes unreliably under
# Lix's cached flake eval after a store GC (the cached eval references the
# toFile store path without re-adding it, so `import` fails with "path ...
# did not exist in the store during evaluation"). An equivalent inline
# eval-time-generated module value exercises the same no-checked-in-source
# vector deterministically.
{ ... }:
{
  imports = [
    ({ ... }: { microvm.mem = 4096; })
  ];
  environment.systemPackages = [ ];
}
