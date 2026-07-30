# Design

## The problem

An agent driving a terminal program has to solve the same problem a human solves
with their eyes: terminals are **stateful**. The byte stream a program emits is
not its output - it is a series of instructions for mutating a screen. `\x1b[2J`
is not text, and a progress bar that redraws with `\r` is one line, not four
hundred.

Three approaches exist:

1. **Give the agent the raw byte stream.** This is what `d2b shell` does today.
   The agent must implement a terminal emulator to make sense of it, or accept a
   corrupted view. Fails immediately on anything using the alternate buffer.
2. **Screen-scrape an existing terminal**, e.g. `tmux capture-pane`. Requires
   tmux, gives a coarse text-only view, and offers no real change signal.
3. **Put a real emulator in the loop.** The agent asks a structured question and
   gets a structured answer.

This prototype takes the third approach, with the constraint that it must not
degrade the human's experience at all.

## Architecture

```
        your keystrokes                                agent
              │                                          │
        /dev/tty (raw)                            unix socket
              │                                          │
              ▼                                          ▼
   ┌──────────────────────────────────────────────────────────┐
   │  tokio::select! pump  (src/pump.rs)                      │
   │                                                          │
   │    tty.read  ──▶ to_child ─┐                             │
   │    sock.recv ──▶ to_child ─┴──▶ PTY master write         │
   │                                                          │
   │    pty.read  ──┬──▶ tty write     (exact bytes, human)   │
   │                └──▶ Session::feed_output (decoded)       │
   │                                                          │
   │    SIGWINCH  ──▶ TIOCGWINSZ ─▶ TIOCSWINSZ + Vt::resize   │
   │    tick(500ms) ─▶ idle checkpoint                        │
   └──────────────────────────────────────────────────────────┘
```

### Why one pump

The human's bytes and the agent's bytes merge into a **single** `to_child`
queue. This is the load-bearing decision in the whole design. Two independent
writers to a PTY master can interleave mid-escape-sequence: an agent injecting
`ESC [ A` while you press a key can produce `ESC [ x A`, which is a different
sequence entirely. One queue, one writer, no interleaving.

The human's screen receives the child's bytes **verbatim**. Only the emulator
sees decoded text. Anything the emulator does not model - sixel, kitty graphics,
an obscure private sequence - still renders correctly for you, because it is
never round-tripped through the emulator on the way to your screen.

### State ownership

`Session` (`src/session.rs`) owns everything derived from child output: the
`avt::Vt`, the mode scanner, the UTF-8 decoder, and the history rings.

It sits behind an `Arc<Mutex<_>>` rather than being message-passed. This is a
deliberate deviation from strict single-ownership: the socket handlers need to
read it, and a request/reply channel would be more code for no benefit at this
scale. The rule that makes it safe is that **the lock is never held across an
`await`**. Every `dispatch` arm takes it, reads or mutates, and drops it before
returning, so a slow or hostile client cannot stall the pump.

## The delta engine

This is the part with actual design content; everything else is plumbing.

`avt::Vt::feed_str()` returns:

```rust
pub struct Changes<'a> {
    pub lines: Vec<usize>,
    pub scrollback: Box<dyn Iterator<Item = Line> + 'a>,
}
```

`ht` discards this entirely. Consuming it is most of the feature - but naively
reporting `lines` is wrong, for a reason worth stating precisely.

### The viewport-index problem

`lines` holds **viewport-relative row indices, not stable row identities**.

On the alternate buffer, the viewport is fixed and does not scroll, so "row 7
changed" means exactly what it appears to mean. On the primary buffer, a single
newline scrolls the whole viewport and marks every row dirty - not because the
content is new, but because different content now occupies those positions.
Reporting that as "22 rows changed" for a one-line `echo` would be technically
true and completely useless.

So the report is mode-aware:

| Mode | `changed_rows` / `rows` | `appended` |
|---|---|---|
| `alt-screen` | dirty-row union, with current text | empty |
| `scrolling` | dirty rows listed, text suppressed | rendered transcript |

In scrolling mode the row *texts* are deliberately withheld, because echoing
back rows whose only change is that content moved through them actively
misleads. The transcript is reconstructed from the LCS diff's additions, which
means it carries `avt`'s rendered text and therefore contains **no escape
sequences** - asserted by
`session::tests::delta_appended_carries_rendered_text_not_escape_sequences`.

### Checkpoints and the baseline

Both modes also carry an LCS line diff against a **checkpoint**: a rendered
viewport snapshot taken periodically (500 ms) and retained in a bounded ring.
The diff is the answer that holds regardless of scrolling, because LCS
recognises that `[a,b,c] → [b,c,d]` is one removal and one addition rather than
three changed rows.

Checkpoints are forced, ignoring the interval, on two events that invalidate
row-index comparison: a **resize** (every row reflows) and an
**alternate-buffer switch** (the screen is replaced wholesale).

#### The idle-baseline bug

An early version took checkpoints only when output arrived. This is subtly
broken, and the end-to-end suite caught it:

> A screen settles at *t*=0.6 s. At *t*=1.0 s the agent asks
> `delta --since 100ms`. The newest checkpoint at or before *t*=0.9 s is the
> **initial, empty** one from session start, because no output arrived after
> *t*=0.6 s to trigger another. The diff reports the entire screen as freshly
> changed - forever, and increasingly wrongly the longer the session idles.

Two fixes, both retained:

1. **An idle checkpoint tick** in the pump, so a settled screen keeps a current
   baseline.
2. **An exact short circuit** in `Session::delta`: every screen change
   originates in child output, and a resize records dirty rows, so if neither
   occurred in the window then *nothing changed*, full stop - returned without
   consulting a checkpoint at all.

The short circuit is the load-bearing one, because it is exact rather than
approximate. The tick improves baseline accuracy for the cases that do change.
Regression coverage: `delta_is_empty_once_output_falls_outside_the_window`,
`delta_still_reports_output_inside_the_window`, `resize_alone_registers_as_a_change`.

### Bounded by construction

All four rings - dirty rows, checkpoints, output, evicted scrollback - are
bounded by count, and the output ring additionally by total bytes. An
agent-facing tool that grows without limit while watching a chatty build is a
memory leak with extra steps. When any ring evicts, `truncated: true` is set on
every subsequent report, so a partial answer is never presented as complete.

## Working around `avt`

Two gaps, handled without forking.

**No alternate-buffer accessor.** `avt` tracks both buffers, but `Vt`'s
`terminal` field is private and there is no `is_alternate()`. `src/modes.rs`
scans the output stream for DECSET/DECRST `?1049`, `?1047` and `?47` instead. It
is an explicit state machine rather than a regex, so it survives a sequence
split across PTY reads - the same hazard the UTF-8 decoder handles for text, and
tested the same way. The scanner also tracks `?2004`, which `avt` does not model
at all and which is needed for bracketed paste regardless. An upstream patch
exposing `active_buffer_type()` would remove half this module.

**`Vt::text()` always reads the primary buffer**, even when the alternate buffer
is active, while `view()` and `lines()` follow the active one. This is a trap if
you assume they agree. It is also useful: it means the shell transcript remains
readable underneath a full-screen TUI, so it is exposed as `primaryText` rather
than hidden. Pinned by `primary_text_survives_an_alt_screen_takeover`.

## Security posture

The socket is `0600` in `$XDG_RUNTIME_DIR`, and the peer's uid is checked with
`SO_PEERCRED` at accept time anyway, mirroring d2b's unsafe-local helper. Mode
bits alone are a weak guarantee if the containing directory is ever wrong.

`bind()` refuses to unlink anything that is not a socket, so a mistyped
`--socket` cannot delete a regular file. Stale sockets from a crashed session
*are* replaced, because otherwise every subsequent start fails.

Request lines are bounded before buffering, so a client that never sends a
newline cannot drive unbounded allocation.

This is a **same-uid convenience boundary, not a privilege boundary**. Anything
that can talk to the socket can type into your terminal.

## What was deliberately not built

- **Detach/reattach.** The session is foreground-only. `dump()` makes reattach
  straightforward to add later, and proving `dump()` was one of the goals, but
  the daemon lifecycle is not needed to answer the questions this lab asks.
- **MCP.** Plain subcommands mean an agent drives this with bash and a human can
  debug it by hand. An MCP shim over the same socket is a thin later addition.
- **Styling.** The screen is reported as text. Colour and attributes are
  available from `avt::Cell` if a use case appears; none has.
- **asciicast output.** The output ring already uses the v3 event shape, so
  writing a `.cast` file is a small addition - but recording is asciinema's job,
  and it does it better.
