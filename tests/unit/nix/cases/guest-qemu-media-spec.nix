# Focused qemu-media Provider option coverage.
{ mkEval, ... }:

let
  positive = mkEval [
    ({ ... }: {
      d2b.qemuMediaRuntime = {
        enable = true;
        kvmRequired = true;
      };
    })
  ];

  invalid = builtins.tryEval (
    (mkEval [
      ({ ... }: {
        d2b.qemuMediaRuntime.qmpReadyTimeoutSeconds = 4;
      })
    ]).config.d2b.qemuMediaRuntime.qmpReadyTimeoutSeconds
  );
in
{
  "guest-qemu-media/defaults" = {
    expr = positive.config.d2b.qemuMediaRuntime;
    expected = {
      enable = true;
      kvmRequired = true;
      qmpReadyTimeoutSeconds = 30;
      qmpOperationTimeoutSeconds = 60;
      runtimeTmpfsQuotaBytes = 10485760;
      runtimeTmpfsQuotaInodes = 1024;
    };
  };

  "guest-qemu-media/invalid-qmp-timeout-rejected" = {
    expr = invalid.success;
    expected = false;
  };
}
