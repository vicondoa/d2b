# Run a QEMU media workload

QEMU media workloads are realm-owned, manual-start workloads. Declare a
runtime provider and bind the workload to it:

```nix
d2b.realms = {
  local-root = {
    path = "local-root";
    placement = "host-local";
  };

  dark = {
    parent = "local-root";
    path = "dark.local-root";
    placement = "host-local";
    allowedUsers = [ "alice" ];
    network = {
      mode = "declared";
      lanSubnet = "10.60.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };

    providers.media = {
      type = "runtime";
      implementationId = "qemu-media";
      configRef = "dark-live-media";
      capabilities = [ "qmp-media-attach" ];
    };

    workloads.dark-live = {
      provider = "media";
      autostart = false;
    };
  };
};
```

Apply the host configuration, then start the canonical target:

```console
$ sudo nixos-rebuild switch --flake .#host
$ d2b up dark-live.dark.local-root.d2b --apply
```

The realm controller starts QEMU paused and owns its pidfd. Media attachment
is resolved from the provider's private `configRef`; do not put host device
paths or transient USB selectors in public workload metadata.

Stop the workload through the same controller:

```console
$ d2b down dark-live.dark.local-root.d2b --apply
```
