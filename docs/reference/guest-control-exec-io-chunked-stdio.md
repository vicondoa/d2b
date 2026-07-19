# Guest control exec I/O

Guest exec uses the authenticated `d2b.guest.v2` ComponentSession. The
historical chunked unary stdio RPC protocol is retired. Exec lifecycle is
typed control RPC; stdin, stdout, stderr, retained logs, and PTY data use
bounded named streams in the same authenticated session.

The control request identifies the operation and its stream. It never carries
terminal bytes, argv, environment, or credentials. The request generation must
equal the ComponentSession generation. Unknown, stale, duplicate, oversized,
or credit-exhausted streams fail closed.

Attached exec streams close on cancellation, deadline expiry, process exit, or
session loss. Detached execution keeps its guest-owned slot and retained log
lifecycle, but log delivery uses a newly authorized named stream. There is no
SSH, old guest-control service, or unary-I/O fallback.

All stream limits are the negotiated ComponentSession limits. Application
consumption grants credit; transport buffering is not treated as delivery.
Closing stdin half-closes only that direction. A reset cancels the associated
attached operation and releases its stream reservation.
