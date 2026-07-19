# Guest control interactive exec

Interactive exec is a typed `d2b.guest.v2` `Exec` operation plus a bounded,
bidirectional ComponentSession named stream. PTY bytes never appear in control
RPCs.

guestd allocates the PTY and starts the configured workload user through the
existing login-session helper. The guest remains the PTY owner. Terminal
resize, allowed signals, stdin half-close, process exit, and cancellation are
ordered stream control messages. Output is one merged PTY stream; a separate
stderr stream is invalid for TTY mode.

Interactive exec is connection-owned and non-durable. Session loss resets its
stream and tears down the exact transient workload unit. Detached TTY exec is
rejected. Reconnect requires a new operation and stream on the current
ComponentSession generation; there is no old ttRPC, chunked-unary, host PTY,
or SSH fallback.

Terminal geometry must remain within the guest's existing bounds. Stream and
operation identifiers are opaque and must not enter logs, metrics, audit
records, or error details.
