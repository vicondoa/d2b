# Destructive-cutover readiness and realm-native schema checks.
{ mkEval, lib, ... }:

let
  base = {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";

    d2b.realms.work = {
      path = "work";
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.desktop.providerRefs.runtime = "runtime";
    };
  };

  unacknowledged = mkEval [ base ];
  acknowledged = mkEval [
    base
    { d2b.acceptDestructiveV2Cutover = true; }
  ];
  cfg = acknowledged.config;
in
{
  "readiness-waves/destructive-ack-defaults-false" = {
    expr = unacknowledged.config.d2b.acceptDestructiveV2Cutover;
    expected = false;
  };

  "readiness-waves/destructive-ack-fails-closed" = {
    expr = lib.any
      (assertion:
        !assertion.assertion
        && lib.hasInfix
          "d2b.acceptDestructiveV2Cutover must be set"
          assertion.message)
      unacknowledged.config.assertions;
    expected = true;
  };

  "readiness-waves/destructive-ack-enables-realm-schema" = {
    expr = lib.all (assertion: assertion.assertion) cfg.assertions;
    expected = true;
  };

  "readiness-waves/legacy-vm-option-absent" = {
    expr = acknowledged.options.d2b ? vms;
    expected = false;
  };

  "readiness-waves/legacy-env-option-absent" = {
    expr = acknowledged.options.d2b ? envs;
    expected = false;
  };

  "readiness-waves/canonical-realm-target" = {
    expr =
      let workload = builtins.head cfg.d2b._index.workloads.enabledList;
      in {
        inherit (workload)
          canonicalTarget
          realmPath
          workloadName
          providerRefs;
        runtimeImplementation =
          workload.providerBindings.runtime.implementationId;
      };
    expected = {
      canonicalTarget = "desktop.work.local-root.d2b";
      realmPath = "work.local-root";
      workloadName = "desktop";
      providerRefs.runtime = "runtime";
      runtimeImplementation = "cloud-hypervisor";
    };
  };
}
