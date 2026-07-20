# Use desktop actions from wlcontrol

**Diataxis category:** how-to.

`d2b-wlcontrol` renders action offers supplied by the desktop observer
composition. Each offer has a local label and an opaque capability.

To invoke an offer:

1. Keep the authenticated `ComponentSession` for `d2b.notify.v2` open.
2. Generate fresh 16-byte request and idempotency identifiers.
3. Set issue and expiry timestamps no more than 120 seconds apart.
4. Build the invocation from the selected offer.
5. Call `NotifyService.InvokeAction` on that same session.
6. Render the returned closed outcome.

Retry an uncertain call with the same idempotency key and capability. The
service returns the recorded outcome without executing the action twice.

Do not decode the capability, append a target, construct a shell command, read
a callback file, or probe another socket. When session establishment or
invocation fails, show the action as unavailable; there is no legacy fallback.
