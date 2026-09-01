# Network host effects are owned by the committed Network resource and the
# root daemon's Host-global admission path. This module is intentionally a
# small module import point: it must not read retired VM or
# caller-provided network values.
{ lib, ... }:

{
  # Keep Network-owned links outside NetworkManager. The broker still owns
  # every create, mutation, and deletion after ownership admission.
  networking.networkmanager.unmanaged = lib.mkAfter [ "interface-name:d2b-*" ];
}
