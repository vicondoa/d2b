{ config, lib, ... }:

let
  cfg = config.nixling;
in
{
  # ---------------------------------------------------------------------------
  # P4 C3/H5: Per-VM dedicated system users for GPU + audio sidecars.
  # Each per-VM sidecar runs as its own dedicated user
  # (`nixling-<vm>-{gpu,snd,swtpm}`), NOT the host's Wayland user.
  # The `nixling.site.launcherUsers` list controls who gets the
  # `nixling-launcher` group (and thus the polkit grant on the
  # framework's units).
  # ---------------------------------------------------------------------------
  users.groups = {
    # nixling-launcher: members of this group get the polkit grant
    # to start/stop/restart the framework's own systemd units. Add
    # users to it via `nixling.site.launcherUsers`.
    nixling-launcher = { };
  } // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-gpu" { })
    (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-snd" { })
    (lib.filterAttrs (_: vm: vm.enable && vm.audio.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-swtpm" { })
    (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-runner" { })
    (lib.filterAttrs (_: vm: vm.enable) cfg.vms));

  users.users =
    # nixling-launcher group membership for any user the site
    # declares. We ONLY add the supplementary group — the user
    # must already exist (declared elsewhere in the consumer's
    # NixOS config). The assertions module enforces that.
    (lib.genAttrs cfg.site.launcherUsers (_: {
      extraGroups = [ "nixling-launcher" ];
    }))
    // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-gpu" {
      isSystemUser = true;
      group = "nixling-${name}-gpu";
      # kvm: needs /dev/kvm to run cloud-hypervisor + crosvm device gpu.
      # security-r2-1: "audio" removed — nixling-<vm>-gpu talks audio ONLY via
      # the vhost-device-sound socket from nixling-<vm>-snd; direct /dev/snd/*
      # access bypasses vhost mediation.  PAM rtprio limits are not needed here
      # because the GPU sidecar itself does no audio mixing.
      # E: nixling-launcher NOT in extraGroups — a compromised sidecar
      # must not be a polkit launcher principal. Only `launcherUsers`
      # (typically the Wayland user) needs nixling-launcher.
      # The per-VM runner group is used for host-side relay sockets that
      # only the matching VM runner should reach.
      extraGroups = [ "kvm" "nixling-${name}-runner" ];
      description = "nixling GPU+hypervisor sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-snd" {
      isSystemUser = true;
      group = "nixling-${name}-snd";
      # audio: PAM rtprio limits for the PipeWire audio thread.
      extraGroups = [ "audio" ];
      description = "nixling audio sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.audio.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-swtpm" {
      isSystemUser = true;
      group = "nixling-${name}-swtpm";
      description = "nixling swtpm emulator for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms));
}
