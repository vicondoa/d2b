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
#   - nixlingMigrateOwnership   — repair orphan swtpm-state UIDs after
#                                 service-user renames, gated on
#                                 `tpm.enable` and skipped for running VMs.
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
in
{
  # v1.1-P5: per-sidecar-user traversal ACL on /var/lib/nixling.
  # /var/lib/nixling itself is `0750 root nixlingd` (set in
  # host-daemon.nix tmpfiles); this activation script grants the
  # documented sidecar users the `--x` traversal bit on the parent
  # dir so they can reach the per-VM subdirectories owned by their
  # respective uid/gid. Without these ACLs, the 0750 parent mode
  # blocks traversal for users not in the `nixlingd` group (which
  # is most sidecar users — they're per-VM-scoped and never in
  # nixlingd group).
  #
  # The enumeration mirrors the user list documented in
  # `docs/reference/privileges.md` § "v1.1-P5 state-dir ACL
  # contract". Each entry is a `--x` (execute-only / traversal)
  # grant — the sidecar user can `chdir` into the directory but
  # not read its contents; per-VM subdirectories under it have
  # their own ACLs scoped to the same sidecar user (see
  # nixlingVmStatePerms above).
  #
  # Idempotent: setfacl overwrites the named entries; running
  # multiple times produces the same final ACL set.
  system.activationScripts.nixlingStateDirAcl = lib.stringAfter [ "users" ] ''
    set -u
    state_dir=/var/lib/nixling
    if [ ! -d "$state_dir" ]; then
      exit 0
    fi
    # Re-assert canonical mode + ownership (defense-in-depth
    # against any 0755 chmod workaround a previous v0.x install
    # may have left behind).
    chown root "$state_dir" || true
    chgrp nixlingd "$state_dir" || true
    chmod 0750 "$state_dir" || true
    # Per-sidecar-user traversal grants. Each grant is `--x`
    # (chdir only); the per-VM subdir under it owns its own
    # read/write ACL.
    # `microvm` is a system user (created by host.nix /
    # nixos.users); grant `u:microvm:--x`.
    ${pkgs.acl}/bin/setfacl -m "u:microvm:--x" "$state_dir" 2>/dev/null || true
    # `kvm` is a Linux GROUP (not a user) — every sidecar that
    # opens /dev/kvm is in this group. Grant `g:kvm:--x` so any
    # group member can traverse the state-dir parent. Treating
    # `kvm` as a user (the v1.1-rc1 bug) silently grants nothing
    # because no `kvm` user exists.
    ${pkgs.acl}/bin/setfacl -m "g:kvm:--x" "$state_dir" 2>/dev/null || true
    # Per-VM sidecar users: enumerated by mapAttrs over cfg.vms
    # — each VM contributes gpu/swtpm/audio/video users that
    # may need traversal.
  '' + lib.concatStringsSep "\n" (lib.mapAttrsToList
    (name: _: ''
      for suffix in gpu swtpm audio video; do
        user="nixling-${name}-$suffix"
        if id "$user" >/dev/null 2>&1; then
          ${pkgs.acl}/bin/setfacl -m "u:$user:--x" /var/lib/nixling 2>/dev/null || true
        fi
      done
    '')
    cfg.vms);

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
          if systemctl is-active --quiet "nixling-${name}-gpu.service" 2>/dev/null \
             || systemctl is-active --quiet "microvm@${name}.service" 2>/dev/null; then
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

  # TPM VMs need an explicit traverse ACL for the dedicated swtpm user on
  # the parent VM state dir, regardless of whether the VM is graphics-backed
  # or headless. The swtpm StateDirectory itself is 0700-owned by the swtpm
  # user; this parent-dir grant is only for the path walk into `swtpm/`.
  system.activationScripts.nixlingTpmStatePerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -d /var/lib/nixling/vms/${name} ]; then
          # v0.1.4 fix: nixling-${name}-swtpm needs +x on the parent
          # state dir to traverse into its `swtpm/` subdir (where
          # systemd's StateDirectory= places it). Without this grant
          # the swtpm service starts but fails to open
          # tpm2-00.permall with EACCES, libtpms enters failure mode,
          # and the VM boots with a freshly-initialised TPM —
          # triggering Entra/Intune device-tampering alerts for
          # tenant-enrolled VMs.
          ${pkgs.acl}/bin/setfacl -m "u:nixling-${name}-swtpm:--x" /var/lib/nixling/vms/${name} || true
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms)));

  # Non-graphics VMs (net VMs) also need microvm:kvm ownership on var.img.
  # No ACL needed: net VMs have no nixling-<vm>-gpu sidecar user.
  system.activationScripts.nixlingNetVmVarImgPerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -f /var/lib/nixling/vms/${name}/var.img ]; then
          if systemctl is-active --quiet "microvm@${name}.service" 2>/dev/null; then
            echo "nixling: ${name} (net VM) is running; skipping var.img ownership fix"
          else
            chown microvm:kvm /var/lib/nixling/vms/${name}/var.img || true
            chmod 0660 /var/lib/nixling/vms/${name}/var.img || true
          fi
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && !vm.graphics.enable) cfg.vms)));

  system.activationScripts.nixlingMigrateOwnership = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: vmCfg: ''
        # Repair orphan swtpm owners/groups in per-VM TPM state after
        # user-name renames. GNU coreutils reports unmapped owners as
        # UNKNOWN rather than the raw UID/GID, so treat any UNKNOWN
        # user/group as orphaned and repair it without disturbing
        # already-correct inodes.
        ${lib.optionalString vmCfg.tpm.enable ''
          if [ -d /var/lib/nixling/vms/${name}/swtpm ]; then
            if systemctl is-active --quiet "nixling-${name}-gpu.service" 2>/dev/null \
               || systemctl is-active --quiet "microvm@${name}.service" 2>/dev/null; then
              echo "nixling: ${name} is running; skipping ownership repair"
            else
              find /var/lib/nixling/vms/${name}/swtpm -mindepth 1 \
                \( -type d -o -type f \) -exec ${pkgs.bash}/bin/bash -c '
                  for f do
                    owner=$(stat -c "%U:%G" "$f" 2>/dev/null || true)
                    case "$owner" in
                      ""|UNKNOWN:*|*:UNKNOWN)
                        chown nixling-${name}-swtpm:nixling-${name}-swtpm "$f" || true
                        echo "nixling: repaired orphan ownership on $f"
                        ;;
                    esac
                  done
                ' bash {} +
            fi
          fi
        ''}
      '')
      (lib.filterAttrs (_: vm: vm.enable) cfg.vms)));
}
