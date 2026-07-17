{ config, lib, generation ? 1, ... }:

let
  rows = import ../realm-storage-rows.nix {
    inherit config generation lib;
  };
in
{
  providers = rows.providers;
}
