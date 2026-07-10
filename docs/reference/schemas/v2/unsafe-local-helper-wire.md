# `unsafe-local-helper-wire.json` schema (`v2`)

Schema: [`unsafe-local-helper-wire.json`](./unsafe-local-helper-wire.json)

This schema captures private helper protocol version 1 between `d2bd` and the
same-UID unsafe-local user helper. Peer credentials, not payload fields,
establish execution identity.

## Contract notes

- Control frames use bounded `AF_UNIX` `SOCK_SEQPACKET` messages.
- Terminal readiness transfers exactly one connected `AF_UNIX` `SOCK_STREAM`.
- Socket and received descriptors use the frozen CLOEXEC requirements.
- Requests contain no uid, environment, cwd, compositor path, or
  public-supplied argv.
