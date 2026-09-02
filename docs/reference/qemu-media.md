# QEMU media Provider contract

**Diataxis category:** reference.

QEMU media is a Provider-backed Guest runtime. It is selected by a Zone-local
`Provider` Resource and uses the daemon/broker control plane for launch,
media access, readiness, and shutdown.

## Resource shape

```nix
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
```

The Guest evaluator is supplied through
`d2b.guestSystems.work.dark-live`. Media selectors are private Provider
configuration or authenticated runtime evidence. Public Resource specs and
status never contain device paths, serial numbers, bus IDs, image paths, or
QEMU arguments.

## Lifecycle

```text
d2b guest start dark-live --zone work --dry-run
d2b guest start dark-live --zone work --apply
d2b guest status dark-live --zone work
d2b guest stop dark-live --zone work --apply
```

The Guest controller waits for the Provider assignment and required child
Resources. The QEMU Provider owns its runner, QMP session, media fd passing,
and provider-aware graceful shutdown. `--force` skips only that graceful wait;
it does not bypass ownership, generation, or finalizer checks.

## Security

Media fds remain broker-local until the Provider's typed operation authorizes
them. The runner receives only its declared device and namespace scope.
Foreign ownership, unsafe paths, malformed media, missing identity evidence,
and uncertain cleanup fail closed.

See [the QEMU media how-to](../how-to/qemu-media.md),
[the CLI contract](./cli-contract.md), and
[the privileges reference](./privileges.md).
