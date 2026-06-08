# tests/smoke-eval-tpm.nix — regression test for Spec correction #35
# (v0.1.4: swtpm-user ACL grant on the VM's parent state dir).
#
# Mirrors tests/smoke-eval.nix but declares one TPM-enabled VM and
# deep-seqs `system.activationScripts.nixlingVmStatePerms.text`.
# The activation snippet under exercise:
#
#   setfacl -m "u:nixling-<vm>-swtpm:--x" /var/lib/nixling/vms/<vm>
#
# Why: the per-VM `nixling-<vm>-swtpm.service` runs as a dedicated
# system user (`nixling-<vm>-swtpm`) and stores its state under
# `/var/lib/nixling/vms/<vm>/swtpm/`. systemd's StateDirectory= sets
# the leaf perms, but the parent directory (`/var/lib/nixling/vms/<vm>`)
# is owned `microvm:kvm 2770` — the swtpm user is neither, so it
# cannot traverse into the leaf. Without the `--x` ACL grant, swtpm
# starts but EACCES'es on `tpm2-00.permall`, libtpms enters failure
# mode, and the VM boots with a freshly-initialised TPM —
# triggering Entra/Intune device-tampering alerts for tenant-enrolled
# VMs.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  flake = builtins.getFlake (toString ./..);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  nixos = nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = {
          device = "tmpfs";
          fsType = "tmpfs";
        };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        # Graphics + TPM enabled VM. The framework's
        # host-activation.nix iteration that emits the parent-dir
        # ACL grant filters on `graphics.enable = true` (the snippet
        # lives in the graphics-VM block), then conditionally adds
        # the `nixling-<vm>-swtpm:--x` line when `tpm.enable = true`.
        # Both toggles are required to reach the assertion below.
        nixling.vms.tpm-vm = {
          enable = true;
          env = "work";
          index = 12;
          ssh.user = "alice";
          graphics.enable = true;
          tpm.enable = true;
          config = {
            networking.hostName = lib.mkDefault "tpm-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };

  activationText =
    nixos.config.system.activationScripts.nixlingVmStatePerms.text;

  # Substring check. The literal `nixling-` prefix and
  # `-swtpm:--x` suffix bracket the VM name without depending
  # on exact whitespace/quoting (which Nix string interpolation
  # already settles deterministically; we just don't want the
  # test to be brittle on minor formatting tweaks).
  expectedFragment =
    ''setfacl -m "u:nixling-tpm-vm-swtpm:--x" /var/lib/nixling/vms/tpm-vm'';

  hasFragment =
    let
      el = builtins.stringLength expectedFragment;
      tl = builtins.stringLength activationText;
      scan = i:
        if i + el > tl then false
        else if (builtins.substring i el activationText) == expectedFragment then true
        else scan (i + 1);
    in scan 0;

  _check =
    if hasFragment
    then null
    else throw ("smoke-eval-tpm: system.activationScripts.nixlingVmStatePerms.text "
                + "does not contain the swtpm parent-dir ACL grant for tpm-vm "
                + "(Spec correction #35 / v0.1.4). Expected fragment:\n  "
                + expectedFragment);
in
  builtins.deepSeq _check
    (builtins.deepSeq activationText
      nixos.config.system.build.toplevel)
