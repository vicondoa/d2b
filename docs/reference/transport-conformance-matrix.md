# Transport conformance matrix

**Diataxis category:** reference.

Transport providers give constellation peers a bidirectional byte stream.
They do not authenticate users, authorize operations, or multiplex named
streams; those checks happen in the peer-session and stream-mux layers.

Every transport adapter should pass the shared conformance checks in
`d2b-constellation-transport::conformance` before it is wired into a
gateway or remote node.

| Behavior | Required result | Loopback status | Local TCP status |
| --- | --- | --- | --- |
| Listen once | A provider exposes one listener per registration; a duplicate listener is rejected with a typed transport error. | Covered | Covered |
| Connect/accept | A successful connect pairs with one accept and returns two independent bidirectional byte streams. | Covered | Covered |
| Byte exactness | Bytes written in either direction arrive unchanged and never cross into another session. | Covered | Covered |
| Queue capacity | A bounded pending-session queue refuses excess connects instead of allocating unbounded state. | Covered | Not applicable: no user-space accept queue. |
| Concurrent sessions | Multiple accepted sessions remain isolated and can be drained deterministically by callers. | Covered | Covered |
| Shutdown | After shutdown, new connects and accepts return a typed unavailable error. | Covered | Covered |
| Frame cap | Peer sessions reject declared or outbound frames above the 1 MiB cap before allocating payload buffers. | Covered | Transport-independent peer-session coverage. |
| Truncated frames | A short read after a valid length prefix is reported as a malformed frame. | Covered | Transport-independent peer-session coverage. |
| Capability negotiation | Peers select the intersection of advertised capabilities, including the empty set. | Covered | Transport-independent peer-session coverage. |
| Stream backpressure | A sender without outbound credit receives a typed backpressure error before bytes are sent. | Covered | Transport-independent stream-mux coverage. |
| Cancellation retry | Repeated cancel of the same stream is idempotent and not treated as a protocol violation. | Covered | Transport-independent stream-mux coverage. |

Future transports can add adapter-specific tests, but they must preserve
the same external behavior: bounded memory, typed refusal on unavailable
or overloaded paths, no byte corruption, and no implicit fallback to a
different transport.
