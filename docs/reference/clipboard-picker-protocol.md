# Clipboard picker service

**Diataxis category:** reference.

The picker uses the frozen `d2b.clipboard.picker.v2` service over an
authenticated local ComponentSession. The generated protobuf service contract
is the authoritative private service definition:

| Method | Mutating | Purpose |
| --- | --- | --- |
| `ListOffers` | no | Return bounded display metadata for offers addressed to one canonical target. |
| `SelectOffer` | yes | Select one offer for the exact canonical destination. |
| `CancelSelection` | yes | Cancel one active selection for the exact canonical destination. |
| `Cancel` | no | Apply the common cancellation contract. |

Mutating requests require a bounded idempotency key. Every request is bound to
the authenticated session generation and deadline. Unknown, malformed,
expired, cross-generation, or target-mismatched input is rejected.

## Transport and authentication

`d2b-clipd` supervises the picker and gives it a connected local transport
descriptor. The descriptor is used only to establish ComponentSession with:

- service package `d2b.clipboard.picker.v2`;
- endpoint purpose `clipboard-picker`;
- endpoint role `clipboard-picker`; and
- the frozen local transport, limit, and attachment policy.

The picker has no socket pathname, launch token, or custom version negotiation.
ComponentSession owns packet framing, authentication, reconnect generation,
deadlines, and request bounds. Clipboard picker requests carry no attachments;
clipboard transfer descriptors are never sent to the picker.

## Typed selections

Offer, selection, and operation identifiers are opaque, non-empty ASCII values
of at most 64 bytes. A list returns no more than 64 offers. The retained offer
set is bounded, as are MIME types, application labels, previews, and optional
PNG thumbnails.

Workload destinations and sources use parsed canonical
`<workload>.<realm-path>.d2b` targets. Host-origin offers use the explicit host
target variant rather than an empty or guessed workload string. Selection and
cancellation require an exact destination match; display labels, application
ids, titles, and workspace names never supply routing authority.

## Picker-visible metadata

The picker may receive:

- opaque offer id;
- canonical source and destination identity;
- closed provider kind for workload targets;
- allowlisted MIME type and optional byte count;
- bounded plain-text preview;
- optional capped PNG thumbnail;
- bounded source-application label;
- closed attribution and capability-preflight states;
- observation and expiry times; and
- whether confirmation is required.

Preview text rejects terminal control sequences and non-display controls.
Metadata that exceeds a bound is rejected rather than truncated into a
different meaning. Expired offers and offers for another destination are not
listed. A denied or unknown capability preflight cannot be selected.

The picker never receives clipboard payload bytes, transfer descriptors,
`NIRI_SOCKET`, data-control authority, or policy authority.
