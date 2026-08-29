# Observability component aggregator.
#
# Imported into the host's NixOS config from `nixos-modules/default.nix`.
# Per-VM imports (`./guest.nix`) and obs-VM-only imports (`./stack.nix`)
# happen elsewhere (via `host.nix` and `observability-vm.nix`
# respectively), gated on per-VM and per-framework toggles. This file
# is the HOST-side import path.
{ ... }:

{
  imports = [
    ./host.nix
  ];
}
