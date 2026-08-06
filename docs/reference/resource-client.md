# Resource client attachment contract

`d2b-resource-client` is the caller-side typed facade for the v3 Zone
resource plane. `ProcessAttachClient<R, C, W>` reuses the same
`TargetResolver`, `ZoneSessionConnector`, retry driver, cancellation token,
and injected `WallClock` as `ZoneClient`.

## Allowed targets

`ProcessAttachTarget` accepts exactly one of:

- an existing `EphemeralProcess/<name>` in an exact Zone; or
- a configured `Process/<name>` launcher in an exact Zone.

Target construction checks the ResourceType and does not authorize the
operation. The authenticated ComponentSession adapter must independently
authorize the fixed process attachment operation for its authoritative
subject. The client accepts no subject, uid, gid, argv, environment, cwd,
credential, admission proof, or lifecycle capability.

## Attach and stream behavior

`ProcessAttachClient::attach` resolves the target through the Zone service
route, checks the authenticated session pin against that route, and asks the
session adapter to open the named stream. The adapter is the integration point
for `ComponentSessionDriver`; it owns session authorization, stream allocation,
credit accounting, transport I/O, and workload-user resolution from the
Process or EphemeralProcess resource. The attach request never selects a user
or carries an execution admission result.

TTY requests require a non-empty initial geometry. Non-TTY requests reject
geometry. One stream message is non-empty and no larger than the negotiated
ComponentSession logical-message ceiling. `ProcessAttachStream::close` and
`cancel` are idempotent and release the session-owned stream; dropping the
wrapper does not perform asynchronous I/O.

The call driver bounds the wall-clock lifetime and total attempts. A
pre-dispatch transport failure or a peer `Immediate` retry verdict may be
retried within that budget. `Never` and `Reauthorize` authorization outcomes,
protocol failures, ambiguous outcomes, deadline expiry, and cancellation are
terminal. Cancellation is forwarded to the authenticated session request and
does not open a replacement stream.

## Identity and routing boundary

Routes require an explicit carriage selection and an exact Zone-scoped owner.
The connector supplies authenticated peer evidence and a session pin; a
different Zone, service, or carriage is refused before the attach adapter
receives the request. Reconnect generation and transcript checks remain
session-adapter responsibilities and stay inside the authenticated pin. The
client exposes neither the underlying session driver nor any descriptor,
socket, host path, store path, or token.
