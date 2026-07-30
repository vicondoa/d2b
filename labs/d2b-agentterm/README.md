# `d2b-agentterm`

*A headless browser, but for terminals - without taking the terminal away from you.*

Run a TUI through `d2b-agentterm` and nothing changes for you: you interact with
it normally, at full speed, with resize, colour and every keystroke intact.
Meanwhile a real VT emulator runs alongside, and an agent can ask it what is on
screen, what changed in the last ten seconds, and send keystrokes of its own.

```
d2b-agentterm run -- lazygit          # you drive it, as usual

d2b-agentterm screen                   # agent: what's on screen?
d2b-agentterm delta --since 10s        # agent: what changed?
d2b-agentterm wait-idle --for 5s       # agent: tell me when it settles
d2b-agentterm keys Down Down Enter     # agent: drive it
```

## Status

**Prototype / spike.** This crate is deliberately *not* a member of the d2b
workspace, is not referenced by `tests/`, the `Makefile` or `flake.nix`, and
changes no shipping d2b component. It exists to validate an architecture before
any of it is proposed for production.

---

# Part 1: Why this exists

## The problem

An agent driving a terminal program has to solve the problem a human solves with
their eyes: terminals are **stateful**. The byte stream a program emits is not
its output - it is a series of instructions for mutating a screen. `\x1b[2J` is
not text. A progress bar that redraws with `\r` is one line, not four hundred.
A full-screen TUI paints over itself continuously, and the "output" of `lazygit`
is not a transcript at all.

Three approaches exist:

1. **Give the agent the raw byte stream.** This is what `d2b shell` does today.
   The agent must implement a terminal emulator to make sense of it, or accept a
   corrupted view. It fails immediately on anything using the alternate buffer.
2. **Screen-scrape an existing terminal**, e.g. `tmux capture-pane`. Requires
   tmux, gives a coarse text-only view, and offers no real change signal.
3. **Put a real emulator in the loop.** The agent asks a structured question and
   gets a structured answer.

This prototype takes the third approach, under a hard constraint: it must not
degrade the human's experience at all.

## The specific question for d2b

`d2b shell` already has attach/detach, but it has **no screen model**. Its
`OutputRing` (`packages/d2b-unsafe-local-helper/src/output_ring.rs`) is a
512 KiB `VecDeque<u8>` of raw bytes with a byte cursor. There is no grid, no
cursor, no alternate-buffer tracking. Re-attach works by replaying those bytes
so that *your* terminal reconstructs the screen.

That is correct for a human and useless for an agent, which has nowhere to
replay them to. This lab answers two questions:

1. What does it take to give a d2b shell a screen model an agent can read?
2. Is `avt::Vt::dump()` a better re-attach mechanism than raw replay?

See "Findings for d2b" at the end.

---

# Part 2: Architecture

```
        your keystrokes                                agent
              |                                          |
        /dev/tty (raw)                            unix socket
              |                                          |
              v                                          v
   +----------------------------------------------------------+
   |  tokio::select! pump   (src/pump.rs)                     |
   |                                                          |
   |    tty.read  --> to_child --+                            |
   |    sock.recv --> to_child --+--> PTY master write        |
   |                                                          |
   |    pty.read  --+--> tty write     (exact bytes, human)   |
   |                +--> Session::feed_output (decoded)       |
   |                                                          |
   |    SIGWINCH  --> TIOCGWINSZ --> TIOCSWINSZ + Vt::resize  |
   |    tick(500ms) --> idle checkpoint                       |
   +----------------------------------------------------------+
```

## Module map

| File | Responsibility |
|---|---|
| `src/main.rs` | Process entry. Builds the Tokio runtime, exits with the child's status. |
| `src/cli.rs` | Clap surface. `run` starts a session; every other subcommand is a one-shot socket client. |
| `src/pump.rs` | The I/O loop. Sole owner of the terminal, the PTY and the session. |
| `src/tty.rs` | `/dev/tty` in raw mode, with guaranteed restoration. `TIOCGWINSZ`. |
| `src/pty.rs` | `forkpty`, `execvpe`, `TIOCSWINSZ`, child reaping. |
| `src/session.rs` | Owns the emulator and the history. Answers `screen` and `delta`. |
| `src/screen.rs` | Rendering `avt` state into agent-readable text. |
| `src/history.rs` | Five bounded, timestamped rings. |
| `src/delta.rs` | The delta report and the LCS line diff. |
| `src/modes.rs` | DEC private mode scanner: alternate buffer, bracketed paste. |
| `src/utf8.rs` | Incremental UTF-8 decoding across read boundaries. |
| `src/keys.rs` | tmux-style key grammar and escape-sequence encoding. |
| `src/protocol.rs` | The JSON wire types. |
| `src/server.rs` | The unix socket: peer checks, framing, dispatch. |

## Why one pump

The human's bytes and the agent's bytes merge into a **single** `to_child`
queue. This is the load-bearing decision in the whole design.

Two independent writers to a PTY master can interleave mid-escape-sequence: an
agent injecting `ESC [ A` while you press a key can produce `ESC [ x A`, which
is a different sequence entirely. One queue, one writer, no interleaving.

The human's screen receives the child's bytes **verbatim**. Only the emulator
sees decoded text. Anything the emulator does not model - sixel, kitty graphics,
an obscure private sequence - still renders correctly for you, because it is
never round-tripped through the emulator on the way to your screen.

## State ownership

`Session` (`src/session.rs`) owns everything derived from child output: the
`avt::Vt`, the mode scanner, the UTF-8 decoder, and the history rings.

It sits behind an `Arc<Mutex<_>>` rather than being message-passed. This is a
deliberate deviation from strict single-ownership: the socket handlers need to
read it, and a request/reply channel would be more code for no benefit at this
scale. The rule that makes it safe is that **the lock is never held across an
`await`**. Every `dispatch` arm takes it, reads or mutates, and drops it before
returning, so a slow or hostile client cannot stall the pump.

---

# Part 3: The delta engine

This is the part with actual design content. Everything else is plumbing.

`avt::Vt::feed_str()` returns:

```rust
pub struct Changes<'a> {
    pub lines: Vec<usize>,
    pub scrollback: Box<dyn Iterator<Item = Line> + 'a>,
}
```

`ht` discards this entirely. Consuming it is most of the feature - but naively
reporting `lines` is wrong, for a reason worth stating precisely.

## The viewport-index problem

`lines` holds **viewport-relative row indices, not stable row identities**.

On the alternate buffer the viewport is fixed and does not scroll, so "row 7
changed" means exactly what it appears to mean. On the primary buffer, a single
newline scrolls the whole viewport and marks every row dirty - not because the
content is new, but because different content now occupies those positions.
Reporting that as "22 rows changed" for a one-line `echo` would be technically
true and completely useless.

So the report is mode-aware:

| Mode | `changedRows` / `rows` | `appended` |
|---|---|---|
| `alt-screen` | dirty-row union, with current text | empty |
| `scrolling` | dirty rows listed, text suppressed | rendered transcript |

In scrolling mode the row *texts* are deliberately withheld, because echoing
back rows whose only change is that content moved through them actively
misleads. The transcript is reconstructed from the LCS diff's additions, which
means it carries `avt`'s rendered text and therefore contains **no escape
sequences**.

## Checkpoints and the baseline

Both modes also carry an LCS line diff against a **checkpoint**: a rendered
viewport snapshot taken periodically (500 ms) and retained in a bounded ring.
The diff is the answer that holds regardless of scrolling, because LCS
recognises that `[a,b,c] -> [b,c,d]` is one removal and one addition rather than
three changed rows.

Checkpoints are forced, ignoring the interval, on two events that invalidate
row-index comparison: a **resize** (every row reflows) and an
**alternate-buffer switch** (the screen is replaced wholesale).

## Bug found during development: the idle baseline

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
approximate. Regression coverage:
`delta_is_empty_once_output_falls_outside_the_window`,
`delta_still_reports_output_inside_the_window`,
`resize_alone_registers_as_a_change`.

## Bug found during development: cursor traffic is not activity

A second, subtler failure showed up while using this against a real agent TUI.

`avt` deliberately does **not** mark a line dirty for pure cursor movement.
An application that repositions its cursor on a timer therefore emits PTY
traffic continuously while the screen stays visually identical. Keying idle
detection off `outputBytes` reports such a session as **busy forever**.

The report therefore separates three distinct signals:

| Field | Meaning |
|---|---|
| `outputBytes` | Raw traffic. Includes cursor and mode changes. |
| `contentChanged` | Whether any row's rendered content differs. **Use this for idle.** |
| `cursorMoved` | Whether the cursor position changed. |

`is_idle()` is `!content_changed`. `cursor_only_activity()` is
`!content_changed && output_bytes > 0`, and the human renderer says so
explicitly: `(no change; 4096 bytes of cursor/control traffic moved nothing
visible)`.

Worth noting what this does *not* cover: a cursor **blink** rendered by your
real terminal emulator (a DECSCUSR blink style) produces no PTY output at all.
It is a property of the display, not the application, so this tool cannot see it
and does not need to - it can never cause a false "busy". Measured against
opencode: 8 seconds of `outputBytes=0` with a static cursor at row 64, col 5.

## Bounded by construction

All five rings - dirty rows, checkpoints, output, evicted scrollback, cursor -
are bounded by count, and the output ring additionally by total bytes. An
agent-facing tool that grows without limit while watching a chatty build is a
memory leak with extra steps. When any ring evicts, `truncated: true` is set on
every subsequent report, so a partial answer is never presented as complete.

Note that `truncated` is deliberately **sticky**: it means "eviction has
happened at some point in this session", not "eviction affected this specific
window". That is the conservative direction, but it does mean a long-running
session reports `truncated: true` permanently.

---

# Part 4: Working around `avt`

Two gaps, handled without forking.

## No alternate-buffer accessor

`avt` tracks both buffers, but `Vt`'s `terminal` field is private and there is
no `is_alternate()`. `src/modes.rs` scans the output stream for DECSET/DECRST
`?1049`, `?1047` and `?47` instead.

It is an explicit state machine rather than a regex, so it survives a sequence
split across PTY reads - the same hazard the UTF-8 decoder handles for text, and
tested the same way (`sequence_split_across_feeds_is_still_recognised`). The
scanner also tracks `?2004`, which `avt` does not model at all and which is
needed for bracketed paste regardless.

An upstream patch exposing `active_buffer_type()` would remove half this module.

## `Vt::text()` always reads the primary buffer

`Vt::text()` reads the primary buffer even when the alternate buffer is active,
while `view()` and `lines()` follow the active one. This is a trap if you assume
they agree.

It is also useful: it means the shell transcript remains readable *underneath* a
full-screen TUI, so it is exposed as `primaryText` rather than hidden. Pinned by
`primary_text_survives_an_alt_screen_takeover`.

---

# Part 5: Usage

## Running a session

```sh
d2b-agentterm run -- bash
d2b-agentterm run -- vim /etc/hosts
d2b-agentterm run --size 100x30 -- htop
d2b-agentterm run --socket /tmp/my.sock -- lazygit
```

The socket path defaults to `$XDG_RUNTIME_DIR/d2b-agentterm-<pid>.sock` and is
printed to stderr at startup (suppress with `--quiet`). Client commands find it
automatically when exactly one session is running; otherwise pass `--socket`, or
set `D2B_AGENTTERM_SOCKET`.

## Driving it as an agent

Every client command is one connect/request/response, so an agent invokes them
as ordinary shell commands. Add `--json` to any of them for machine output.

| Command | Purpose |
|---|---|
| `screen` | The current viewport, one line per row |
| `delta --since 10s` | What changed over a trailing window |
| `wait-idle --for 5s` | Block until content stops changing |
| `keys Enter Down C-c` | Send key names |
| `text 'git status'` | Send literal text |
| `raw $'\e[A'` | Send raw bytes; escape hatch |
| `resize --cols 100 --rows 30` | Advisory resize |
| `info` | Size, buffer, modes, pid, uptime |
| `dump` | A sequence that reconstructs the screen |

### `wait-idle`

The primitive an agent actually wants after submitting input:

```sh
d2b-agentterm wait-idle --for 5s --timeout 120s --await-change
```

`--await-change` requires the screen to change at least once before it can be
considered idle, so a screen that has not started reacting yet is not mistaken
for one that has already settled. Exit code `0` on idle, `2` on timeout.

Keyed on `contentChanged`, not `outputBytes`, for the reason in Part 3.

## Key names

tmux-style, inherited from `ht`:

```
Enter Space Escape Tab BackTab Backspace Insert Delete
Left Right Up Down Home End PageUp PageDown F1..F12
^x   C-x   S-x   A-x   M-x
```

Modifiers combine in any order: `C-S-Left`, `S-A-Up`, `C-A-S-Right`. Anything
unrecognised **without** a modifier is sent as literal text, so
`keys nano Enter` works. Anything unrecognised **with** a modifier is an error
rather than being typed into your shell as garbage.

Arrow keys automatically follow the child's DECCKM state, emitting `ESC[A` or
`ESC O A` as appropriate. Getting this wrong is the single most common reason
arrows appear to "not work" in a TUI.

### Scrolling

`PageUp` / `PageDown` are sent to the **application**, which is correct for
alternate-buffer TUIs that own their own scrollback (opencode, lazygit, `less`).
Verified end to end against `less`: `1 -> 20 -> 39 -> 20`.

For an ordinary scrolling shell the real terminal handles scrollback locally and
the shell ignores PageUp - but you do not need to scroll at all there, because
`primaryText` already exposes the full primary buffer including scrollback.

---

# Part 6: Sourcing and licences

| Source | Licence | Used how |
|---|---|---|
| [`avt`](https://github.com/asciinema/avt) 0.18 | Apache-2.0 | Cargo dependency. The emulator. |
| [`ht`](https://github.com/andyk/ht) 0.4.0 | Apache-2.0 | **Vendored:** the key table, in `src/keys.rs`. See `NOTICE`. |
| [asciinema CLI](https://github.com/asciinema/asciinema) | **GPL-3.0** | **Design reference only. No code copied.** |

The asciinema CLI is GPL-3.0-or-later, incompatible with this crate's
Apache-2.0. Its passthrough-recorder *architecture* was used as a reference and
reimplemented; nothing was copied. `avt` itself is Apache-2.0 and is a normal
dependency.

## Three bugs in `ht` that are fixed here

`ht`'s PTY layer was rewritten rather than vendored, because:

1. **Resize never reached the kernel.** `ht`'s `Command::Resize` calls only
   `vt.resize()`; there is no `TIOCSWINSZ`. The child keeps its startup size
   forever, so every full-screen TUI renders at the wrong size after a resize.
   `e2e.sh` section 3 asserts the fix by having the child run `tput cols`.
2. **Double-owned master descriptor.** `ht` builds both a `File::from_raw_fd`
   and an `AsyncFd<OwnedFd>` over the same fd, so both close it.
3. **Per-chunk UTF-8 decoding.** `String::from_utf8_lossy` on each PTY read
   corrupts any multi-byte character split across a read boundary.
   `src/utf8.rs` carries the partial tail forward instead.

Also added: bracketed paste (`ht` has none), `Insert`/`Delete`/`BackTab`,
order-insensitive modifiers on every key, and consumption of the `Changes`
value `ht` discards - which is the whole delta feature.

---

# Part 7: Security posture

The socket is `0600` in `$XDG_RUNTIME_DIR`, and the peer's uid is checked with
`SO_PEERCRED` at accept time anyway, mirroring d2b's unsafe-local helper. Mode
bits alone are a weak guarantee if the containing directory is ever wrong.

`bind()` refuses to unlink anything that is not a socket, so a mistyped
`--socket` cannot delete a regular file. Stale sockets from a crashed session
*are* replaced, because otherwise every subsequent start fails.

Request lines are bounded before buffering, so a client that never sends a
newline cannot drive unbounded allocation.

Bracketed-paste payloads have any embedded `ESC[201~` end-marker stripped, so a
hostile string cannot terminate its own paste and inject the remainder as
keystrokes.

**This is a same-uid convenience boundary, not a privilege boundary.** Anything
that can talk to the socket can type into your terminal.

---

# Part 8: Building, testing, and known limits

## Building

No `rustup` is required, but note the `rust-toolchain.toml` pin is only honoured
when `cargo` is a rustup shim; with a plain toolchain it is inert and documents
intent only. Built and tested here on rustc 1.95.0.

```sh
cargo build
cargo test          # 119 unit tests, hermetic
bash e2e.sh         # 33 end-to-end checks against real programs
cargo deny check    # optional; not wired into any gate
```

`e2e.sh` allocates a PTY with `script`, so it runs anywhere with a `/dev/tty` -
it does not need an interactive shell.

## Test coverage

`cargo test` - 119 hermetic unit tests.

`bash e2e.sh` - 33 checks against real programs under a real PTY:

| Section | What it proves |
|---|---|
| 1 | `bash` scrolling mode, delta transcript, quiet-window detection |
| 2 | `vim` on the alternate buffer, alt-screen delta mode |
| 3 | Resize reaches the child, verified with `tput cols` |
| 4 | `dump` reconstructs screen state |
| 5 | `C-c` interrupt, refusal of unencodable keys |
| 6 | `PageUp`/`PageDown` scrolling in `less` |
| 7 | `wait-idle`, `contentChanged`, `cursorMoved` |
| 8 | Socket permissions and cleanup |

## Known limits

- **Contention.** You and the agent share one input queue, so bytes never
  interleave mid-sequence - but nothing stops you both typing at once. An
  advisory lock is the obvious next step.
- **Size conflicts.** The PTY follows *your* window. `resize` is advisory and
  says so in its response; your next `SIGWINCH` overrides it.
- **`avt` fidelity.** It replicates no specific terminal and does not implement
  sixel or kitty graphics. Fine for TUIs, not for image protocols.
- **`truncated` is sticky**, as described in Part 3.
- **No detach/reattach.** The session is foreground-only. `dump()` makes this
  straightforward to add later.
- **No styling.** The screen is reported as text. Colour and attributes are
  available from `avt::Cell` if a use case appears; none has.

---

# Part 9: Findings for d2b

Neither is proposed as a d2b change here; this lab exists to justify them.

**1. `Vt::dump()` is a better re-attach than raw replay.** `d2b shell attach`
replays `OutputRing` from cursor 0, so once the 512 KiB ring wraps, the attach
begins mid-escape-sequence and renders a corrupt screen. `dump()` emits a
bounded sequence that reconstructs the exact current screen - cursor, pen,
margins, charset and alternate-buffer state included. Verified by
`session::tests::dump_reconstructs_alt_screen_state`, which feeds a dump into a
fresh emulator and asserts the viewports match.

**2. An emulator in the supervisor is purely additive.** `OutputRing` keeps
serving the human attach path unchanged; a `Vt` fed from the same byte stream is
what makes a d2b shell agent-legible. That would be a `ShellOp::Screen` /
`ShellOp::Delta` addition to terminal protocol v1.
