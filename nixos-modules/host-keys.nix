{ lib, ... }:

{
  # Realm and workload key directories are broker-created from generated
  # storage IDs. Nix activation never generates, stages, chmods, chowns, or
  # repairs key material below the fixed /var/lib/d2b anchor.
  system.activationScripts.d2bGenerateKeys = lib.stringAfter [ "users" ] ''
    :
  '';
}
