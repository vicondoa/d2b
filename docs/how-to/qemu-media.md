# Configure the QEMU media Provider

QEMU media is a Provider-backed Guest runtime. The Zone owns the Provider and
Guest Resources; the Guest controller owns lifecycle and readiness while the
Provider owns QEMU and media effects through the broker.

## Declare the resources

```nix
{
  d2b.artifacts.qemu-media-provider = {
    package = inputs.d2bQemuMedia.packages.${system}.default;
    type = "provider";
  };

  d2b.zones.work.resources.runtime-qemu-media = {
    type = "Provider";
    spec = {
      artifactId = "qemu-media-provider";
      config.controllerExecutionRef = "Host/host";
    };
  };

  d2b.zones.work.resources.dark-live = {
    type = "Guest";
    spec = {
      providerRef = "Provider/runtime-qemu-media";
      systemArtifactId = "dark-guest-system";
    };
  };
}
```

The Guest system evaluator is supplied separately:

```nix
d2b.guestSystems.work.dark-live = {
  config.system.build.toplevel = inputs.darkGuestSystem;
};
```

Physical media and removable devices are selected by the authenticated
Provider operation using opaque runtime evidence. Do not place `/dev` paths,
serial numbers, bus IDs, or host locators in the Guest spec or shared
configuration.

## Start and inspect

```bash
d2b guest start dark-live --zone work --dry-run
d2b guest start dark-live --zone work --apply
d2b guest status dark-live --zone work
d2b device usb probe --zone work
```

The dry-run and status output show only bounded Resource, Provider, and
readiness metadata. QEMU process arguments, media paths, credentials, and
broker handles remain private.

## Stop

```bash
d2b guest stop dark-live --zone work --apply
```

Stop is provider-aware and dependency-ordered. `--force` skips only the
graceful Provider wait; it does not bypass identity fencing or finalization.
