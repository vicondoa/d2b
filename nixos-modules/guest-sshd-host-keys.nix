# Guest-side wiring for host-managed sshd host keys.
#
# Point sshd at the keys provisioned on the host by
# `nixos-modules/host-ssh-host-keys.nix` and shared in via the
# `nl-ssh-host` virtiofs tag from `nixos-modules/store.nix`. Disable
# the NixOS default `ssh-keygen -A` activation hook so sshd does not
# try to generate keys into the read-only nix store at boot.
#
# Imported into every enabled VM's NixOS config via host.nix's
# per-VM imports list.
{ config, lib, ... }:

{
  # microvm.nix wires the share — see store.nix. Guest just consumes.
  services.openssh = lib.mkIf config.services.openssh.enable {
    # Disable the upstream NixOS activation script that runs
    # `ssh-keygen -A` and writes into /etc/ssh/. Without this the
    # guest spends ~10s per boot generating throwaway keys it then
    # ignores. (sshd itself respects `hostKeys` and only uses ours.)
    hostKeys = lib.mkForce [{
      type = "ed25519";
      path = "/run/nixling-sshd-host-keys/ssh_host_ed25519_key";
    }];
  };

  # Block the legacy NixOS host-key bootstrap. Without this the
  # activation script tries to populate /etc/ssh/ssh_host_*_key from
  # ssh-keygen at activation time, which fails on the read-only nix
  # store and prints a noisy error every boot.
  systemd.tmpfiles.rules = [
    # Mirror the on-host path inside the guest at /etc/ssh/ for
    # operator habit-of-eye: `cat /etc/ssh/ssh_host_ed25519_key.pub`
    # works the same way it always has, but the file is a symlink
    # into the virtiofs share. This is purely cosmetic; sshd reads
    # the canonical path above.
    "L+ /etc/ssh/ssh_host_ed25519_key.pub - - - - /run/nixling-sshd-host-keys/ssh_host_ed25519_key.pub"
  ];
}
