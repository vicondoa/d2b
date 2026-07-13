# 0010. Wire protocol and typed errors

- Status: Superseded by [ADR 0045](0045-provider-and-transport-framework.md)
- Date: 2026-05-26
- Wave: W2
- Plan slice: "Daemon API and state model (wire-frozen in W2 even though much is unimplemented)."
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), [ADR 0007](0007-bash-coexistence-and-migration.md), [ADR 0008](0008-supported-platforms-and-rejected-targets.md)

## Context

W2 introduces a real daemon/broker boundary, but the implementation is
still intentionally narrow: read-only public commands, a minimal broker
surface, and no host mutation. Even so, the wire protocol becomes the
new compatibility boundary for the CLI, daemon, and later supervisor.

That boundary needs two things immediately:

1. a transport and negotiation model that can survive future waves
   without breaking every client; and
2. an error model that automation can consume without scraping
   shell text.

The security review also narrowed the audit read path: non-root callers
must not read the broker's audit file directly, even when they are
allowed to request its contents through the daemon.

## Decision

1. **Transport.** W2 uses non-abstract Unix-domain `SOCK_SEQPACKET`
   sockets for both `/run/d2b/public.sock` and `/run/d2b/priv.sock`.
   Each message is framed as a 4-byte little-endian length prefix plus
   one JSON body, with a maximum frame size of 1 MiB.
2. **Handshake and downgrade policy.** Every connection begins with a
   `Hello` handshake containing a `SemverRange` plus feature flags. The
   server may select only a version inside the advertised range; any
   downgrade outside that range is rejected explicitly.
3. **Closed enums and strict decoding.** W2 treats documented message
   enums as closed. Unknown fields inside a known message type are
   rejected (`deny_unknown_fields`) rather than ignored.
4. **Forward compatibility.** Unknown feature flags are ignored during
   negotiation so newer clients can probe older servers safely, but that
   is the only permissive forward-compatibility rule at this layer.
5. **Typed errors.** Operator-visible failures use a typed envelope with
   a stable `Kind`, a stable exit code, a redacted message, a
   remediation hint, a docs anchor, and an `owningCommand` field.
   Messages must not leak secrets, stack traces, or incidental host
   paths.
6. **Audit read path.** The only supported audit read path is CLI →
   daemon → broker `ExportBrokerAudit`. Non-root callers never read the
   audit file directly.

## Consequences

1. Positive: `SOCK_SEQPACKET` gives message-boundary preservation and
   keeps the daemon/broker API local-only by construction.
2. Positive: explicit version negotiation lets W2 freeze the wire shape
   now while still leaving room for later additive features.
3. Positive: strict decoding prevents "partially understood request"
   behavior on security-sensitive paths.
4. Positive: typed errors give automation stable handles (`kind`, exit
   code, docs anchor) instead of shell-text scraping.
5. Positive: the audit path stays least-privilege — the broker keeps the
   write fd, the daemon mediates reads, and non-root callers never get a
   raw file descriptor.
6. Negative: additive protocol work now has to go through explicit
   version/feature design rather than opportunistic extra fields.
7. Negative: older permissive serde habits are no longer acceptable on
   this boundary; type changes must be reviewed as protocol changes.

## Alternatives considered

- **`SOCK_STREAM` plus ad hoc read loops:** rejected because W2 wants
  record boundaries preserved and does not benefit from stream semantics.
- **Ignoring unknown fields everywhere:** rejected because it hides
  client/server drift on a security-sensitive API.
- **Free-form string errors only:** rejected because operators and tests
  need a stable machine-consumable failure surface.
- **Letting the CLI read the broker audit file directly:** rejected
  because it widens the privilege boundary and bypasses daemon-side
  authorization.

## References

- plan.md, "W2: Rust workspace and API skeleton"
- [docs/reference/daemon-api.md](../reference/daemon-api.md)
- [docs/reference/error-codes.md](../reference/error-codes.md)
- [docs/explanation/state-lock.md](../explanation/state-lock.md)
