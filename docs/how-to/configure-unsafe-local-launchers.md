# Configure an unsafe-local launcher

Use unsafe-local only for applications trusted to run as the requesting host
user. It is a Provider-backed `Host` Resource with no VM isolation.

## Declare the Host resource

```nix
d2b.zones.local-root.resources.tools = {
  type = "Host";
  spec = {
    providerRef = "Provider/system-core";
    isolationPosture = "none";
  };
};
```

The current launcher contract carries a typed Host target, item ID, and
operation ID. Private Provider configuration resolves the executable and
environment; public Resource specs and status never carry configured argv,
credentials, host paths, or unit names.

## Use the launcher and shell

```bash
d2b launch Host/tools --item browser
d2b shell open Host/tools --name terminal
d2b shell attach ShellSession/terminal
```

The helper verifies the requester UID at every session boundary and accepts
only the exact terminal fd and bounded output cursor for that session.

## Verify

```bash
d2b resource status Host/tools --zone local-root
d2b shell list --zone local-root
d2b audit --json
```

Unsafe-local status must visibly report the `none` isolation posture. Missing
Provider capability, stale identity, or an unavailable helper is a typed
failure; the CLI does not fall back to a Guest or SSH.
