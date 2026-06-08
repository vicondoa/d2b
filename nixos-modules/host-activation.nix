# Activation scripts for nixling.
#
# Phase 2b removed the three legacy activation scripts that used to
# live here. If you're upgrading from a pre-public version of nixling
# (pre-Phase-2b), see CHANGELOG.md for the manual migration steps:
#
#   * nixlingSbctlBackup    — host-specific (maintainer's sbctl pipeline);
#                             no public framework concern. Move
#                             *-backup.tar.gz files out of $HOME by hand
#                             if you ever ran the maintainer setup.
#   * nixlingStoreChownRepair — one-shot fix for a past chown bug (P5
#                             round-1 leaked group=kvm into /nix/store
#                             inodes via the per-VM hardlink farm).
#                             If you ran a pre-Phase-2b nixling, run
#                             the repair script from the historical
#                             /etc/nixos commit once, then forget about
#                             it. New installs are unaffected.
#   * nixlingMigrateState   — one-shot renamer (/var/lib/microvms →
#                             /var/lib/nixling/vms, plus /var/lib/swtpm
#                             → vms/<vm>/swtpm). New installs land on
#                             the new layout from the start. Pre-W3
#                             consumers should use the Phase 9
#                             migration script.
#
# What remains here:
#   - nixlingVmStatePerms       — per-graphics-VM ACLs on the state dir
#                                 + var.img + every *.img disk so the
#                                 nixling-<vm>-gpu sidecar user (not
#                                 microvm) can read/write them.
#   - nixlingNetVmVarImgPerms   — net VMs use microvm:kvm 0660 on var.img.
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
in
{
  # Per-graphics-VM state dir + var.img + extra-disk ownership.
  # nixling-<vm>-gpu runs CH + crosvm-gpu, so var.img is locked to 0600
  # and ACL-granted to nixling-<vm>-gpu (the host user can no longer read it).
  # The state dir remains microvm:kvm 2770 so other microvm.nix tooling works.
  # Skips running VMs to avoid disrupting open file descriptors.
  # Idempotent.
  system.activationScripts.nixlingVmStatePerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -d /var/lib/nixling/vms/${name} ]; then
          chown microvm /var/lib/nixling/vms/${name} || true
          chgrp kvm /var/lib/nixling/vms/${name} || true
          chmod 2770 /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -m "g::r-x" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -m "u:nixling-${name}-gpu:rwx" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "g::r-x" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "u:nixling-${name}-gpu:rw" /var/lib/nixling/vms/${name} || true
          # security-r8-audio-13: iterate over EVERY *.img disk in the
          # state dir, not just var.img. microvm.nix VMs can declare
          # arbitrary additional volumes — all of them are owned by
          # microvm:kvm and read by CH (running as nixling-${name}-gpu).
          # Without per-image ACLs the sidecar user can traverse the
          # directory but gets EACCES on open(). The default ACL on the
          # parent dir only applies to NEW files; pre-existing disks
          # keep their original perms and need an explicit setfacl.
          if pgrep -f "nixling-${name}-gpu\|microvm@${name}\b" >/dev/null 2>&1; then
            echo "nixling: ${name} is running; skipping disk image ownership fix (apply on next nixling up)"
          else
            for img in /var/lib/nixling/vms/${name}/*.img; do
              [ -f "$img" ] || continue
              chown microvm:kvm "$img" || true
              chmod 0600 "$img" || true
              ${pkgs.acl}/bin/setfacl -m "u:nixling-${name}-gpu:rw" "$img" || true
            done
          fi
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms)));

  # Non-graphics VMs (net VMs) also need microvm:kvm ownership on var.img.
  # No ACL needed: net VMs have no nixling-<vm>-gpu sidecar user.
  system.activationScripts.nixlingNetVmVarImgPerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -f /var/lib/nixling/vms/${name}/var.img ]; then
          if pgrep -f "microvm@${name}\b" >/dev/null 2>&1; then
            echo "nixling: ${name} (net VM) is running; skipping var.img ownership fix"
          else
            chown microvm:kvm /var/lib/nixling/vms/${name}/var.img || true
            chmod 0660 /var/lib/nixling/vms/${name}/var.img || true
          fi
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && !vm.graphics.enable) cfg.vms)));
}
