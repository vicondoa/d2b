# Cancellation safety

The depth for the cancellation section of `SKILL.md`: the precise definition,
the `select!` trap, the common-operation table, timeouts as cancellation, the two
structural fixes, graceful shutdown, cancellation tokens, and `JoinSet`.

## The definition, precisely

A future is cancellation-safe if dropping it before completion leaves no work
half-done and loses no data already taken from a source. The drop is not an
error the code handles — it is the future going away mid-operation, with no
cleanup running. Cancellation safety is a property of the operation, not of the
runtime: the runtime will drop futures whether you ask it to or not.

## The `select!` trap

`select!` polls every branch and completes on the first one ready; the branches
that were not ready are dropped. A branch that takes input and then awaits
something irreversible loses the input when another branch wins:

```rust,ignore
// `queue.pop()` runs when the branch future is built, before the race. If
// shutdown wins, `process` is dropped — and the popped item is gone.
tokio::select! {
    _ = process(queue.pop()) => {}
    _ = shutdown.cancelled() => break,
}
```

The fix is to take the input *before* the race, so a drop cannot reach it:

```rust,ignore
let item = queue.pop(); // outside the select: dropping a branch cannot lose it
tokio::select! {
    _ = process(item) => {}
    _ = shutdown.cancelled() => {
        queue.push_back(item); // or hand it to the next task
        break;
    }
}
```

The same shape covers any "read, then do something irreversible" branch. And
when one branch must always win the race — shutdown, or a control channel —
`tokio::select! { biased => ... }` fixes the priority instead of leaving it to
polling luck.

## Which common operations are safe

| Operation | Cancellation-safe? | Why |
|---|---|---|
| `tokio::time::sleep` | Yes | Dropping the future cancels the timer; nothing was consumed |
| `mpsc::Receiver::recv` | Yes | A dropped `recv` does not take the message; the next `recv` still gets it |
| `AsyncReadExt::read` | Yes | A dropped read leaves the stream where it was; bytes already copied are owned by the caller |
| `AsyncReadExt::read_exact` | Conditionally | A partial read is buffered inside the reader, so it is safe only when the *same* reader is resumed — a protocol frame assembled from the partial data is where the bug lives |
| `tokio::sync::Mutex::lock` | Yes | Dropping a pending lock removes the waiter; the lock state is unchanged |
| A database transaction future | No | Commit and rollback state lives on the server; a dropped future may have executed statements the caller does not know about, on a connection in an uncommitted state |

The pattern in the unsafe rows: the operation has effects outside the future —
on a server, on a connection, on a popped queue — and dropping the future does
not undo those effects.

## Timeouts as cancellation

`tokio::time::timeout` drops the inner future when the deadline passes. Wrapping
a non-cancellation-safe operation in a timeout is therefore a data-loss bug,
not a safety net: the timeout did not make the operation safe, it scheduled its
mid-flight drop. If the operation is not safe, make it safe first (below), then
wrap it.

## The two structural fixes

**Make the irreversible step atomic.** Order the operations so the drop point
sits between things where a retry is harmless — write-then-acknowledge, never
acknowledge-then-write:

```rust,ignore
// The append is irreversible and runs first. A drop between the two loses the
// acknowledgement, not the work — the sender retries, and the append is
// idempotent on the job id.
async fn submit(job: Job) -> Result<()> {
    durable.append(job).await?;
    ack.send(job.id()).await
}
```

**Make the operation resumable.** Keep an explicit checkpoint — which items are
done, where the frame ends, which statement the transaction is on — so a
dropped future can be restarted from the checkpoint instead of from zero. The
checkpoint is data the caller owns, not state inside the future.

## Graceful shutdown

`CancellationToken` (from `tokio_util`) or a `watch` channel is the standard
shape: every long-lived task selects its work against the token, and shutdown
cancels the token once. The failure mode this prevents is the one where a
spawned task ignores shutdown and keeps running past it — when the runtime
shuts down, the task is dropped at its current await point, which is exactly an
uncontrolled cancellation of whatever it was mid-flight on. A task that checks
the token is dropped cleanly; one that does not is cancelled, with all the
data-loss the table above describes.

## Cancellation tokens

Mechanically, the token is a shared flag with a `.cancelled()` future, so a task
selects it alongside its real work and shuts down at a point it chose, with
cleanup running on the way out. The contrast is dropping a `JoinHandle`: that
aborts the task at an arbitrary `.await` and gives the task no chance to
release anything it holds. To cancel a subtree of tasks, derive child tokens
from the parent (`child_token()`); cancelling the parent then cancels every
child. And the rule that bounds the whole approach: a token check is only as
good as the interval between checks — a worker that checks once a second cannot
stop mid-second, so the interval must stay shorter than the deadline the
shutdown actually has.

```rust,ignore
// The task picks its own exit point and gets to clean up.
tokio::select! {
    result = do_work() => result,
    _ = token.cancelled() => cleanup().await,
}
```

## JoinSet

`JoinSet` for a dynamic group of tasks: completion is observed as it happens,
and dropping the set aborts the rest — an uncontrolled cancellation of every
task still running, with no chance to clean up.
