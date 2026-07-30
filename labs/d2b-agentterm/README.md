# `d2b-agentterm` - an agent-drivable terminal (prototype)

*A headless browser, but for terminals - without taking the terminal away from you.*

Run a TUI through `d2b-agentterm` and nothing changes for you: you interact with
it normally, at full speed, with resize and colours and every keystroke intact.
Meanwhile a real VT emulator runs alongside, and an agent can ask it what is on
screen, what changed in the last ten seconds, and send keystrokes of its own.

```
d2b-agentterm run -- lazygit          # you drive it, as usual

d2b-agentterm screen                   # agent: what's on screen?
d2b-agentterm delta --since 10s        # agent: what changed?
d2b-agentterm keys Down Down Enter     # agent: drive it
```

## Status

**Prototype / spike.** This crate is deliberately *not* a member of the d2b
workspace, is not referenced by `tests/`, the `Makefile` or `flake.nix`, and
changes no shipping d2b component. It exists to validate an architecture before
any of it is proposed for production. See `DESIGN.md`.

## Why this exists

`d2b shell` already has attach/detach, but it has no screen model: its
`OutputRing` is a 512 KiB ring of raw bytes, and re-attach works by replaying
those bytes so that *your* terminal reconstructs the screen. That is correct for
a human and useless for an agent, which has nowhere to replay them to.

This prototype answers two questions for d2b:

1. What does it take to give a d2b shell a real screen model an agent can read?
2. Is `avt::Vt::dump()` a better re-attach mechanism than raw replay?

See "Findings for d2b" below.

## Building

No `rustup` is required, but note the `rust-toolchain.toml` pin is only honoured
when `cargo` is a rustup shim; with a plain toolchain it is inert and documents
intent only. Built and tested here on rustc 1.95.0.

```sh
cargo build
cargo test          # 116 unit tests, hermetic
bash e2e.sh         # 24 end-to-end checks against real programs
```

`e2e.sh` allocates a PTY with `script`, so it runs anywhere with a
`/dev/tty` - it does not need an interactive shell.

## Usage

### Running a session

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

### Driving it as an agent

Every client command is one connect/request/response, so an agent invokes them
as ordinary shell commands. Add `--json` to any of them for machine output.

| Command | Purpose |
|---|---|
| `screen` | The current viewport, one line per row |
| `delta --since 10s` | What changed over a trailing window |
| `keys Enter Down C-c` | Send key names |
| `text 'git status'` | Send literal text |
| `raw $'\e[A'` | Send raw bytes; escape hatch |
| `resize --cols 100 --rows 30` | Advisory resize |
| `info` | Size, buffer, modes, pid, uptime |
| `dump` | A sequence that reconstructs the screen |

### Key names

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

## The delta

"What changed" has no single honest answer, because `avt` reports changed rows
as **viewport-relative indices, not stable row identities**. On a fixed
full-screen TUI that is exactly right. On a scrolling shell it is misleading:
every row goes dirty on every newline simply because content moved through it.

So the report is mode-aware:

- **`alt-screen`** - the dirty-row union with each row's current text. Precise
  and small. This is what you want for `lazygit`, `htop`, `vim`.
- **`scrolling`** - the appended transcript, reconstructed from rendered text so
  it carries no escape sequences.

Both carry a real LCS line diff against a checkpoint from the start of the
window, which is the answer that survives scrolling either way.

Two things are reported rather than papered over:

- **`alt_screen_switched`** - a buffer switch inside the window makes row
  indices incomparable. You get told, not silently diffed across.
- **`truncated`** - a history ring evicted, so the answer may be partial.

Live example against `htop`, which redraws continuously:

```
window 3.0s  mode alt-screen
changed rows: 22
   1 |     0[||||||||||   46.1%]   3[|||||||||||| 55.6%]  ...
   4 |   Mem[||||||||||||||||||22.5G/62.5G] Tasks: 175, 1675 thr
   6 |                                       Uptime: 2 days, 09:38:25
  10 | 3143882 paydro  20  0 77.9G 1305M R  69.3  2.0  5h42:20 ...
```

Rows 0, 5, 7, 8 and 9 - the static header, blank lines and column titles - are
correctly absent.

## Sourcing and licences

| Source | Licence | Used how |
|---|---|---|
| [`avt`](https://github.com/asciinema/avt) 0.18 | Apache-2.0 | Cargo dependency. The emulator. |
| [`ht`](https://github.com/andyk/ht) 0.4.0 | Apache-2.0 | **Vendored:** the key table, in `src/keys.rs`. See `NOTICE`. |
| [asciinema CLI](https://github.com/asciinema/asciinema) | **GPL-3.0** | **Design reference only. No code copied.** |

The asciinema CLI is GPL-3.0-or-later, incompatible with this crate's
Apache-2.0. Its passthrough-recorder *architecture* was used as a reference and
reimplemented; nothing was copied. `avt` itself is Apache-2.0 and is a normal
dependency.

### Three bugs in `ht` that are fixed here

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

## Findings for d2b

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

## Known limits

- **Contention.** You and the agent share one input queue, so bytes never
  interleave mid-sequence - but nothing stops you both typing at once. An
  advisory lock is the obvious next step.
- **Size conflicts.** The PTY follows *your* window. `resize` is advisory and
  says so in its response; your next `SIGWINCH` overrides it.
- **`avt` fidelity.** It replicates no specific terminal and does not implement
  sixel or kitty graphics. Fine for TUIs, not for image protocols.
- **No alt-buffer accessor upstream.** `avt` tracks both buffers but exposes
  neither; `src/modes.rs` scans DECSET/DECRST `?1049`/`?1047`/`?47` out of the
  output stream instead. An upstream patch exposing `active_buffer_type()`
  would let half that module go away.
- **`Vt::text()` always reads the primary buffer**, even when the alternate
  buffer is active. That is a trap if you assume it follows `view()` - and a
  gift, because it means the shell transcript stays visible underneath a TUI.
  Exposed as `primaryText`.

## Validation

`cargo test` - 116 hermetic unit tests.

`bash e2e.sh` - 24 checks against real programs under a real PTY: `bash`
scrolling and delta modes, `vim` on the alternate buffer, resize propagation
verified via `tput cols` in the child, `dump` reconstruction, `C-c` interrupt
handling, refusal of unencodable keys, and socket permissions and cleanup.

Last run: 116 passed / 0 failed, 24 passed / 0 failed.
