# Desktop action service

Desktop actions use `NotifyService.InvokeAction` in the
`d2b.notify.v2` service package. The service is available only through an
authenticated `ComponentSession` whose endpoint purpose and role are both
`desktop-observer`.

## Action offer

The observer-side composition may issue an offer for a currently valid action.
An offer contains:

- a closed action kind;
- a 32-byte random capability encoded as 64 lowercase hexadecimal characters;
  and
- presentation text selected locally from the closed action kind.

The capability carries no command, ceremony identifier, VM name, workload
name, or other target. Target authority remains in the bounded server-side
capability store.

Offers expire after 120 seconds. At most 256 live offers are retained. The
service refuses additional offers at capacity rather than silently evicting
live authority.

## Invocation

An invocation contains only:

- a 16-byte request identifier;
- a 16-byte idempotency key;
- the opaque capability;
- issue and expiry timestamps.

The encoded request is limited to 512 bytes and its lifetime cannot exceed 120
seconds. The service rejects unauthenticated sessions, untrusted transports,
wrong service/endpoint contracts, malformed requests, expired requests, and
unknown capabilities.

The capability is consumed before execution. A retry with the same
idempotency key and capability receives the cached closed outcome without
executing again. Reusing the capability with a different key, or reusing a key
for a different capability, fails closed. The replay cache is bounded to 256
records and 120 seconds.

## Outcomes and diagnostics

The only execution outcomes are `succeeded`, `notApplicable`, `denied`, and
`failed`. Errors are fixed codes. Debug output and observability expose only
closed action kinds and counters; they do not expose capabilities, commands,
targets, identifiers, or executor diagnostics.

There is no command-line forwarding, durable-state callback, legacy socket, or
alternate endpoint fallback. If the authenticated component session is
unavailable, actions are unavailable.
