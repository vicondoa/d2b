# Clipboard policy

**Diataxis category:** reference.

D2b clipboard policy is enforced by `d2b-clipd`, not by the picker.

## Defaults

- Payloads are memory-only by default.
- Same-realm compatible paste may use the most recent compatible item without UI
  when policy permits.
- Cross-realm transfer requires an explicit allow policy, a trusted paste-intent
  token, and normally a picker selection.
- Missing intent, missing picker, timeout, crash, audit failure, or policy denial
  closes the target transfer FD without writing.

The initial MIME allowlist is:

- `text/plain;charset=utf-8`
- `text/plain`
- `text/html`
- `image/png`

Custom MIME types, `text/uri-list`, file-manager copied-file formats, and
`application/octet-stream` are denied. File transfer remains a separate feature.
Password-manager hint MIME types such as `x-kde-passwordManagerHint` suppress
plaintext previews and either bypass history or store masked metadata according
to policy.

## Configurable caps

`d2b.site.clipboard.caps.*` declares per-item bytes, total memory bytes,
per-MIME byte caps, picker candidate counts, preview bytes, thumbnail bytes,
held-FD caps, and materialization rate limits. Evaluation fails when total memory
is smaller than the per-item cap, when a MIME cap exceeds the per-item cap, or
when asymmetric protocol frame caps are invalid.

`d2b.site.clipboard.ttl.*` declares history, picker request, paste intent,
pending-FD, and fallback arming timeouts.

## Lifecycle cleanup

VM lock, pause, stop, and destroy cleanup is declared in
`d2b.site.clipboard.policy.cleanup`. Runtime cleanup is driven by explicit
`d2bd` lifecycle events, not by proxy disconnects alone.

## Audit and metrics

Audit records contain metadata only: source realm, destination realm, MIME type,
byte count, decision, attribution quality, timestamp, request id, and bounded
reason code. They never include raw payloads, previews, URLs, HTML, image bytes,
or transfer paths.

Metrics use bounded enum labels only. They may count decisions, active entries,
memory use, held FDs, Niri/bridge status, and latencies. They must not label by
request id, app title, arbitrary app id, URL, preview, or raw MIME outside the
closed allowlist. Formal audit delivery is fail-closed for the associated
transfer; droppable diagnostics and metrics may be coalesced or dropped with a
counter.
