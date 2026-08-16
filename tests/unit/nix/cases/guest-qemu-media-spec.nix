# Focused qemu-media Provider option and Guest dependency coverage.
{ lib, mkEval, ... }:

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

  guestFixture = withKvm: { ... }: {
    d2b.qemuMediaRuntime = {
      enable = true;
      kvmRequired = true;
    };
    d2b.zones.local-root.resources = {
      host = {
        type = "Host";
        spec = { };
      };
      network-local = {
        type = "Provider";
        spec = { };
      };
      volume-local = {
        type = "Provider";
        spec = { };
      };
      device-kvm = {
        type = "Provider";
        spec = {
          config = { };
        };
      };
      runtime-qemu-media = {
        type = "Provider";
        spec = {
          config = {
            controllerExecutionRef = "Host/host";
            networkProviderRef = "Provider/network-local";
            volumeProviderRef = "Provider/volume-local";
          };
        };
      };
      host-kvm = {
        type = "Device";
        spec = {
          providerRef = "Provider/device-kvm";
        };
      };
      media-vm = {
        type = "Guest";
        spec = {
          providerRef = "Provider/runtime-qemu-media";
          systemArtifactId = null;
          deviceAttachments = lib.optional withKvm {
            deviceRef = "Device/host-kvm";
            exclusive = false;
          };
          provider = {
            schemaId = "runtime-qemu-media.d2bus.org/Guest/spec";
            schemaVersion = "1.0";
            settings = { };
          };
        };
      };
    };
  };

  assertionBools = evaluated:
    builtins.map (assertion: assertion.assertion) evaluated.config.assertions;
  falseCount = values: lib.length (lib.filter (value: !value) values);
  kvmRejects = {
    missing = assertionBools (mkEval [ (guestFixture false) ]);
    present = assertionBools (mkEval [ (guestFixture true) ]);
  };
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

  "guest-qemu-media/kvm-required-rejects-missing-device" = {
    expr = falseCount kvmRejects.missing > falseCount kvmRejects.present;
    expected = true;
  };

  "guest-qemu-media/kvm-required-accepts-in-zone-device" = {
    expr = falseCount kvmRejects.present < falseCount kvmRejects.missing;
    expected = true;
  };
}
