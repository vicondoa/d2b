{ lib, ... }:

let
  evaluated = lib.evalModules {
    modules = [ (import ../default.nix) ];
  };
  configured = lib.evalModules {
    modules = [
      (import ../default.nix)
      {
        config.d2b.qemuMediaRuntime = {
          enable = true;
          qmpReadyTimeoutSeconds = 45;
          runtimeTmpfsQuotaBytes = 32 * 1024 * 1024;
        };
      }
    ];
  };
in
{
  cases = {
    "provider-runtime-qemu-media/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b.qemuMediaRuntime true;
      expected = true;
      propagateError = true;
    };

    "provider-runtime-qemu-media/defaults-and-bounds" = {
      expr = {
        enabled = configured.config.d2b.qemuMediaRuntime.enable;
        readyTimeout = configured.config.d2b.qemuMediaRuntime.qmpReadyTimeoutSeconds;
        quota = configured.config.d2b.qemuMediaRuntime.runtimeTmpfsQuotaBytes;
      };
      expected = {
        enabled = true;
        readyTimeout = 45;
        quota = 32 * 1024 * 1024;
      };
    };
  };
}
