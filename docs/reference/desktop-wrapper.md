# Desktop launcher integration

**Diataxis category:** historical reference.

The old generated per-Guest desktop wrapper contract is retired. Current
desktop integrations must call the public Rust CLI or daemon socket and
address typed Zone Resources:

```bash
d2b guest start <name> --zone <zone> --apply
d2b shell open Guest/<name> --zone <zone> --name terminal
```

No wrapper may invoke a shell fallback, a per-Guest systemd unit, a static
manifest lookup, or a raw broker operation. Desktop clients should surface
typed daemon errors and use only bounded status/launcher metadata.

See [`../reference/zone-cli-contract.md`](./zone-cli-contract.md) and
[`../how-to/configure-desktop-terminal-integration.md`](../how-to/configure-desktop-terminal-integration.md).
