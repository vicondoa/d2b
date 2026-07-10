{ config, lib, ... }:

let
  cfg = config.d2b;
  hasUnsafeLocal = lib.any
    (realm:
      realm.enable
      && lib.any
        (workload: workload.enable && workload.kind == "unsafe-local")
        (builtins.attrValues realm.workloads))
    (builtins.attrValues cfg.realms);
in
{
  config = lib.mkIf hasUnsafeLocal {
    boot.kernel.sysctl = {
      "net.core.rmem_max" = lib.mkDefault (512 * 1024);
      "net.core.wmem_max" = lib.mkDefault (512 * 1024);
    };
  };
}
