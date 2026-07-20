{ lib, flakeRoot, ... }:

let
  config =
    import (flakeRoot + "/examples/with-observability/configuration.nix") {
      inherit lib;
    };
  work = config.d2b.realms.work;
in
{
  "examples-with-observability/cutover-acknowledged" = {
    expr = config.d2b.acceptDestructiveV2Cutover;
    expected = true;
  };

  "examples-with-observability/stack-enabled" = {
    expr = config.d2b.observability.enable;
    expected = true;
  };

  "examples-with-observability/work-realm" = {
    expr = {
      inherit (work) path;
      providerType = work.providers.runtime-local.type;
      implementationId =
        work.providers.runtime-local.implementationId;
    };
    expected = {
      path = "work.local-root";
      providerType = "runtime";
      implementationId = "cloud-hypervisor";
    };
  };

  "examples-with-observability/workload-uses-guest-component" = {
    expr = {
      providerRefs = work.workloads.work-app.providerRefs;
      autostart = work.workloads.work-app.autostart;
      imports = map toString work.workloads.work-app.config.imports;
    };
    expected = {
      providerRefs.runtime = "runtime-local";
      autostart = true;
      imports = [
        (toString
          (flakeRoot
            + "/nixos-modules/components/observability/guest.nix"))
      ];
    };
  };

  "examples-with-observability/legacy-vm-env-absent" = {
    expr = {
      hasVms = config.d2b ? vms;
      hasEnvs = config.d2b ? envs;
    };
    expected = {
      hasVms = false;
      hasEnvs = false;
    };
  };
}
