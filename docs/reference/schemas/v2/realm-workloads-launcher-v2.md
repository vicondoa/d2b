# `realm-workloads-launcher-v2.json` schema (`v2`)

Schema: [`realm-workloads-launcher-v2.json`](./realm-workloads-launcher-v2.json)

This argv-free artifact describes provider-neutral workload launcher items,
execution posture, and realm presentation metadata. It is installed
`0640 root:d2bd`; authorized unprivileged clients consume it through the public
daemon API rather than reading the bundle directly.

## Contract notes

- `runtimeState` remains `contract-only` until daemon dispatch is enabled.
- `items` contains generic `exec` and `shell` metadata, never configured argv.
- `realmAccentColor` is presentation metadata and never an authorization input.
- `invariants` freezes the no-secrets, provider-neutral, typed-posture boundary.
