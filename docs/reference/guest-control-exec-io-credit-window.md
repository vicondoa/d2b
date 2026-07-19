# Guest control exec I/O credit

Exec, shell, file, and terminal data use ComponentSession named-stream credit.
The former guest-specific credit-window candidate is retired.

Each direction has independent per-stream and aggregate plaintext ceilings.
Credit is granted only after the application consumes bytes. A producer may
not exceed advertised credit, and one stalled stream cannot consume reserved
control capacity or another stream's credit. Oversized messages, unknown
streams, sequence violations, and credit exhaustion reset the affected stream
and fail the operation closed.

Control RPC, cancellation, keepalive, attachment control, and named-stream
traffic retain the ComponentSession scheduler's fixed priority and fairness
rules. Reconnection never resumes a stream implicitly; an idempotent operation
must authorize a new stream on the current session generation.
