# `closures/<vm>.json` schema (`v2`)

Schema: [`closures.json`](./closures.json)

Each `closures/<vm>.json` artifact records the realised Nix closure for
one VM plus the runner-parity metadata used by the daemon/oracle tests.

## Top-level fields

- `schemaVersion` — schema directory/version for this artifact.
- `vm` — VM name the closure belongs to.
- `toplevel` — declared NixOS system closure path.
- `closurePaths` — complete transitive closure required for that VM.
- `declaredRunner` — runner path emitted by the public manifest.
- `runnerParityPath` — observed/snapshotted runner path used for parity.
- `runnerParityOk` — whether `declaredRunner` and `runnerParityPath` agree.
- `generation` — provenance metadata for the closure snapshot.

## Contract notes

- The runner parity fields let Layer-1 drift gates compare the manifest
  contract against the audited runner shape without re-evaluating the host.
