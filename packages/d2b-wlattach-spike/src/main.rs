//! `d2b-wlattach` — tmux for GUI apps.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand};
use d2b_wlattach_spike::present::Frontend;
use d2b_wlattach_spike::serve::host::{
    ClientState, PublishedSurface, SessionHost, Shadow, ShadowSurface,
};
use smithay::reexports::wayland_server::{Display, ListeningSocket};

#[derive(Parser)]
#[command(
    name = "d2b-wlattach",
    about = "Reconnectable Wayland application forwarding (prototype)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a session, launch an application in it, and attach.
    Run {
        #[arg(short, long)]
        session: Option<String>,
        /// Attach if the session already exists instead of failing.
        #[arg(short = 'A', long)]
        attach_if_exists: bool,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// Show the window again.
    Attach {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Hide the window. The application keeps running.
    Detach {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// List sessions.
    Ls,
    /// Report session state.
    Status {
        #[arg(short, long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Terminate the application and the session.
    Kill {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Internal: the session host process.
    #[command(hide = true)]
    Serve {
        #[arg(long)]
        session: String,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// Internal: the disposable window frontend.
    #[command(hide = true)]
    Present {
        #[arg(long)]
        session: String,
    },
    /// Internal: inject a raw input event, for testing the delivery path.
    #[command(hide = true)]
    Inject {
        #[arg(long)]
        session: String,
        /// Wayland keycode (the evdev code; xkb adds 8 on top). KEY_U is 22.
        #[arg(long)]
        key: Option<u32>,
        /// Inject a pointer click at this position instead of a key.
        #[arg(long, num_args = 2, value_names = ["X", "Y"])]
        click: Option<Vec<f64>>,
        /// Send only the button, with no preceding enter or motion -- the
        /// "clicked without moving the mouse first" case.
        #[arg(long)]
        bare: bool,
    },
}

fn base_dir() -> PathBuf {
    let rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(rt).join("d2b-wlattach")
}

fn session_dir(name: &str) -> PathBuf {
    base_dir().join(name)
}

/// Resolve which session a verb applies to.
///
/// With `-s`, that one. Without, only if exactly one session exists — an
/// ambiguous target lists the candidates and fails rather than guessing.
fn resolve(session: Option<String>) -> Result<String, String> {
    if let Some(s) = session {
        return Ok(s);
    }
    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base_dir()) {
        for e in rd.flatten() {
            if e.path().join("ctl.sock").exists()
                && let Some(n) = e.file_name().to_str()
            {
                found.push(n.to_owned());
            }
        }
    }
    match found.len() {
        0 => Err("no sessions".into()),
        1 => Ok(found.remove(0)),
        _ => {
            found.sort();
            Err(format!("several sessions, use -s: {}", found.join(", ")))
        }
    }
}

fn ctl_send(name: &str, verb: &str) -> Result<String, String> {
    let path = session_dir(name).join("ctl.sock");
    let mut s = UnixStream::connect(&path).map_err(|_| format!("no such session: {name}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(verb.as_bytes()).map_err(|e| e.to_string())?;
    s.shutdown(std::net::Shutdown::Write).ok();
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match real_main() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("d2b-wlattach: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            session,
            attach_if_exists,
            argv,
        } => run(session, attach_if_exists, argv),
        Cmd::Attach { session } => {
            let n = resolve(session)?;
            print!("{}", ctl_send(&n, "attach")?);
            Ok(())
        }
        Cmd::Detach { session } => {
            let n = resolve(session)?;
            print!("{}", ctl_send(&n, "detach")?);
            Ok(())
        }
        Cmd::Kill { session } => {
            let n = resolve(session)?;
            print!("{}", ctl_send(&n, "kill")?);
            Ok(())
        }
        Cmd::Status { session, json } => {
            let n = resolve(session)?;
            print!(
                "{}",
                ctl_send(&n, if json { "status-json" } else { "status" })?
            );
            Ok(())
        }
        Cmd::Ls => {
            let mut any = false;
            if let Ok(rd) = std::fs::read_dir(base_dir()) {
                let mut names: Vec<String> = rd
                    .flatten()
                    .filter(|e| e.path().join("ctl.sock").exists())
                    .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                    .collect();
                names.sort();
                for n in names {
                    any = true;
                    let st = ctl_send(&n, "status").unwrap_or_else(|_| "unreachable\n".into());
                    print!("{n}\t{st}");
                }
            }
            if !any {
                println!("no sessions");
            }
            Ok(())
        }
        Cmd::Serve { session, argv } => serve(&session, &argv),
        Cmd::Present { session } => present(&session),
        Cmd::Inject {
            session,
            key,
            click,
            bare,
        } => inject(&session, key, click, bare),
    }
}

fn run(session: Option<String>, attach_if_exists: bool, argv: Vec<String>) -> Result<(), String> {
    let name = session.unwrap_or_else(|| {
        let base = argv
            .first()
            .and_then(|a| Path::new(a).file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_owned();
        format!("{base}-{}", std::process::id())
    });
    let dir = session_dir(&name);
    if dir.join("ctl.sock").exists() {
        if attach_if_exists {
            return ctl_send(&name, "attach").map(|r| print!("{r}"));
        }
        return Err(format!(
            "session {name} already exists; use `attach`, `kill`, or -A"
        ));
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.arg("serve").arg("--session").arg(&name).arg("--");
    for a in &argv {
        c.arg(a);
    }
    c.stdin(Stdio::null());
    let _ = c.spawn().map_err(|e| e.to_string())?;

    for _ in 0..200 {
        if dir.join("ctl.sock").exists() {
            std::thread::sleep(Duration::from_millis(150));
            print!("{}", ctl_send(&name, "attach")?);
            println!("session {name}: detach with `d2b-wlattach detach -s {name}`");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("session host did not start".into())
}

// ---------------------------------------------------------------- session host

struct Hosted {
    app: Child,
    frontend: Option<Child>,
    attached: bool,
}

fn serve(name: &str, argv: &[String]) -> Result<(), String> {
    let dir = session_dir(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    set_mode(&dir, 0o700)?;

    let mut display: Display<SessionHost> = Display::new().map_err(|e| e.to_string())?;
    let mut dh = display.handle();
    let mut state = SessionHost::new(&dh);

    let wl_path = dir.join("wayland-0");
    let _ = std::fs::remove_file(&wl_path);
    let listener = ListeningSocket::bind_absolute(wl_path.clone()).map_err(|e| e.to_string())?;

    let input_path = dir.join("input.sock");
    let _ = std::fs::remove_file(&input_path);
    let input_listener = UnixListener::bind(&input_path).map_err(|e| e.to_string())?;
    set_mode(&input_path, 0o600)?;
    input_listener.set_nonblocking(true).ok();
    // Several streams may be open at once: the frontend, plus test scaffolding.
    let mut input_conns: Vec<(UnixStream, Vec<u8>)> = Vec::new();

    let ctl_path = dir.join("ctl.sock");
    let _ = std::fs::remove_file(&ctl_path);
    let ctl = UnixListener::bind(&ctl_path).map_err(|e| e.to_string())?;
    set_mode(&ctl_path, 0o600)?;
    ctl.set_nonblocking(true).ok();

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("WAYLAND_DISPLAY", &wl_path);
    cmd.env_remove("DISPLAY");
    let app = cmd.spawn().map_err(|e| format!("launch failed: {e}"))?;

    let mut hosted = Hosted {
        app,
        frontend: None,
        attached: false,
    };
    let shadow = Arc::clone(&state.shadow);
    let mut last_rev = u64::MAX;

    let cleanup = |ctl_path: &Path, wl_path: &Path, name: &str| {
        let _ = std::fs::remove_file(ctl_path);
        let _ = std::fs::remove_file(wl_path);
        let _ = std::fs::remove_file(shadow_path(name));
        let _ = std::fs::remove_file(session_dir(name).join("input.sock"));
    };

    loop {
        if let Ok(Some(stream)) = listener.accept()
            && dh
                .insert_client(stream, Arc::new(ClientState::default()))
                .is_err()
        {
            log::warn!("rejected a client");
        }

        // Accept input streams and drain whatever they have sent.
        while let Ok((s, _)) = input_listener.accept() {
            s.set_nonblocking(true).ok();
            input_conns.push((s, Vec::new()));
        }
        let mut events: Vec<d2b_wlattach_spike::wire::dto::InputEvent> = Vec::new();
        input_conns.retain_mut(|(c, buf)| {
            let mut chunk = [0u8; 4096];
            let mut alive = true;
            loop {
                match c.read(&mut chunk) {
                    Ok(0) => {
                        alive = false;
                        break;
                    }
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            while buf.len() >= 4 {
                let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                if len > 4096 {
                    // Malformed peer: drop the stream rather than trust it.
                    alive = false;
                    break;
                }
                if buf.len() < 4 + len {
                    break;
                }
                let frame: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
                if let Ok(ev) =
                    postcard::from_bytes::<d2b_wlattach_spike::wire::dto::InputEvent>(&frame)
                {
                    events.push(ev);
                }
            }
            alive
        });
        for ev in events {
            d2b_wlattach_spike::serve::host::apply_input(&mut state, ev);
        }

        display.dispatch_clients(&mut state).ok();
        display.flush_clients().ok();

        if let Ok((mut s, _)) = ctl.accept() {
            let mut verb = String::new();
            s.set_read_timeout(Some(Duration::from_millis(500))).ok();
            let _ = s.read_to_string(&mut verb);
            let reply = match verb.trim() {
                "attach" => {
                    if hosted.attached {
                        "already attached\n".to_owned()
                    } else {
                        // Publish current content before the frontend starts so
                        // it has something to show immediately.
                        publish(name, &shadow, &mut last_rev, true);
                        match spawn_frontend(name) {
                            Ok(c) => {
                                hosted.frontend = Some(c);
                                hosted.attached = true;
                                // Focus belongs to the old generation.
                                state.pointer_focused = false;
                                "attached\n".to_owned()
                            }
                            Err(e) => format!("attach failed: {e}\n"),
                        }
                    }
                }
                "detach" => {
                    detach(&mut hosted);
                    "detached\n".to_owned()
                }
                "kill" => {
                    detach(&mut hosted);
                    let _ = hosted.app.kill();
                    let _ = s.write_all(b"killed\n");
                    cleanup(&ctl_path, &wl_path, name);
                    return Ok(());
                }
                v if v.starts_with("close ") => match v[6..].trim().parse::<u64>() {
                    Ok(key) if state.request_close(key) => "close forwarded\n".to_owned(),
                    Ok(_) => "no such window\n".to_owned(),
                    Err(_) => "bad key\n".to_owned(),
                },
                "status" => status(&shadow, &hosted, false),
                "status-json" => status(&shadow, &hosted, true),
                other => format!("unknown verb: {other}\n"),
            };
            let _ = s.write_all(reply.as_bytes());
        }

        if hosted.attached {
            // Run the application.s render clock only while something is
            // actually showing its output.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            d2b_wlattach_spike::serve::host::send_frame_callbacks(&state, now);
            publish(name, &shadow, &mut last_rev, false);
        }

        if let Some(f) = hosted.frontend.as_mut()
            && matches!(f.try_wait(), Ok(Some(_)))
        {
            // The frontend died. The application is untouched — this is exactly
            // the property the prototype exists to demonstrate.
            log::info!("frontend exited; application still running");
            hosted.frontend = None;
            hosted.attached = false;
        }

        // Only the application exiting ends the session. A detach never does.
        if matches!(hosted.app.try_wait(), Ok(Some(_))) || !state.running {
            detach(&mut hosted);
            cleanup(&ctl_path, &wl_path, name);
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(4));
    }
}

fn publish(name: &str, shadow: &Arc<Mutex<Shadow>>, last_rev: &mut u64, force: bool) {
    let Some((rev, tops)) = shadow.lock().ok().map(|g| (g.revision, g.toplevels())) else {
        return;
    };
    if !force && rev == *last_rev {
        return;
    }
    *last_rev = rev;
    if let Err(e) = write_shadow(name, &tops) {
        log::warn!("could not publish shadow: {e}");
    }
}

fn detach(h: &mut Hosted) {
    if let Some(mut f) = h.frontend.take() {
        let _ = f.kill();
        let _ = f.wait();
    }
    h.attached = false;
}

fn spawn_frontend(name: &str) -> Result<Child, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .arg("present")
        .arg("--session")
        .arg(name)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}

fn shadow_path(name: &str) -> PathBuf {
    session_dir(name).join("shadow.bin")
}

fn write_shadow(name: &str, tops: &[(u64, ShadowSurface)]) -> Result<(), String> {
    use d2b_wlattach_spike::serve::host::{PublishedSurface, SnapshotMeta};

    let t0 = std::time::Instant::now();
    let dir = session_dir(name);
    let mut published: Vec<(u64, PublishedSurface)> = Vec::with_capacity(tops.len());

    for (key, s) in tops {
        let meta = match s.snapshot.as_ref() {
            Some(snap) => {
                // Pixels go straight into a per-surface file that the frontend
                // maps as a wl_shm pool. Serialising them into the metadata
                // stream instead cost a full extra copy plus a rename per frame.
                //
                // Written in place and never truncated: the compositor may have
                // this file mapped, and shrinking it under a live mapping would
                // SIGBUS it.
                let px = dir.join(format!("px-{key}.raw"));
                let f = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&px)
                    .map_err(|e| e.to_string())?;
                let want = snap.pixels.len() as u64;
                if f.metadata().map(|m| m.len()).unwrap_or(0) < want {
                    f.set_len(want).map_err(|e| e.to_string())?;
                }
                use std::os::unix::fs::FileExt;
                f.write_all_at(&snap.pixels, 0).map_err(|e| e.to_string())?;
                Some(SnapshotMeta {
                    width: snap.width,
                    height: snap.height,
                    stride: snap.stride,
                    format: snap.format,
                    len: snap.pixels.len() as u64,
                    seq: t0.elapsed().as_nanos() as u64 ^ snap.pixels.len() as u64,
                })
            }
            None => None,
        };
        published.push((
            *key,
            PublishedSurface {
                title: s.title.clone(),
                app_id: s.app_id.clone(),
                meta,
            },
        ));
    }

    let bytes = postcard::to_allocvec(&published).map_err(|e| e.to_string())?;
    let tmp = shadow_path(name).with_extension("tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, shadow_path(name)).map_err(|e| e.to_string())?;
    log::debug!(
        "publish {} metadata bytes in {:?}",
        bytes.len(),
        t0.elapsed()
    );
    Ok(())
}

/// Closed metric vocabulary only: never paths, argv, pids or window titles.
fn status(shadow: &Arc<Mutex<Shadow>>, h: &Hosted, json: bool) -> String {
    let (n, retained) = shadow
        .lock()
        .map(|g| {
            let t = g.toplevels();
            (
                t.len(),
                t.iter().filter(|(_, s)| s.snapshot.is_some()).count(),
            )
        })
        .unwrap_or((0, 0));
    if json {
        format!(
            "{{\"attached\":{},\"windows\":{n},\"retained\":{retained},\"buffer_kind\":\"shm\"}}\n",
            h.attached
        )
    } else {
        format!(
            "{}\twindows={n}\tretained={retained}\n",
            if h.attached { "attached" } else { "detached" }
        )
    }
}

fn set_mode(p: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
}

// ------------------------------------------------------------- window frontend

fn present(name: &str) -> Result<(), String> {
    let conn = wayland_client::Connection::connect_to_env()
        .map_err(|_| "cannot reach the host compositor".to_string())?;
    let (mut fe, mut queue) =
        Frontend::bind(&conn, session_dir(name)).map_err(|e| e.to_string())?;
    let qh = queue.handle();

    // Input flows back to the session host over its own stream.
    let mut input = UnixStream::connect(session_dir(name).join("input.sock")).ok();
    if let Some(s) = input.as_mut() {
        s.set_write_timeout(Some(Duration::from_millis(200))).ok();
    }

    let mut last: Vec<u8> = Vec::new();
    loop {
        if let Ok(bytes) = std::fs::read(shadow_path(name))
            && bytes != last
        {
            last.clone_from(&bytes);
            match postcard::from_bytes::<Vec<(u64, PublishedSurface)>>(&bytes) {
                Ok(tops) => {
                    log::debug!("shadow: {} toplevel(s), {} bytes", tops.len(), bytes.len());
                    let live: Vec<u64> = tops.iter().map(|(k, _)| *k).collect();
                    fe.reconcile(&live);
                    for (key, shadow) in tops {
                        if let Err(e) = fe.upsert(key, &shadow, &qh) {
                            log::warn!("upsert failed: {e}");
                        }
                    }
                }
                Err(e) => log::warn!("shadow decode failed: {e}"),
            }
        }

        // A roundtrip both flushes our requests and delivers the compositor.s
        // reply, which is what drives the configure -> attach -> commit dance.
        if queue.roundtrip(&mut fe).is_err() {
            break;
        }

        // Forward captured input upward.
        if let Some(s) = input.as_mut() {
            let n = fe.input.len();
            if n > 0 {
                log::debug!("forwarding {n} input event(s)");
            }
            for ev in fe.input.drain(..) {
                let Ok(body) = postcard::to_allocvec(&ev) else {
                    continue;
                };
                let len = (body.len() as u32).to_le_bytes();
                if s.write_all(&len).is_err() || s.write_all(&body).is_err() {
                    break;
                }
            }
        } else {
            fe.input.clear();
        }

        // Forward compositor close requests to the session host, which
        // forwards them to the application. The application decides.
        for key in fe.close_requested.drain(..).collect::<Vec<_>>() {
            let _ = ctl_send(name, &format!("close {key}"));
        }

        if !fe.running {
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

/// Inject input straight into the session host's stream.
///
/// Test scaffolding. It exercises the host-to-application delivery path without
/// a virtual-input tool, which installs its own keymap and so produces keycodes
/// that mean nothing under ours.
fn inject(name: &str, key: Option<u32>, click: Option<Vec<f64>>, bare: bool) -> Result<(), String> {
    use d2b_wlattach_spike::wire::dto::InputEvent;
    let mut s = UnixStream::connect(session_dir(name).join("input.sock"))
        .map_err(|e| format!("no input socket: {e}"))?;

    let events: Vec<InputEvent> = if let Some(xy) = click {
        let (x, y) = (
            xy.first().copied().unwrap_or(0.0),
            xy.get(1).copied().unwrap_or(0.0),
        );
        if bare {
            // No enter, no motion: the "clicked without moving the mouse first"
            // case, which used to be silently dropped.
            vec![
                InputEvent::PointerButton {
                    button: 0x110,
                    pressed: true,
                },
                InputEvent::PointerButton {
                    button: 0x110,
                    pressed: false,
                },
            ]
        } else {
            vec![
                InputEvent::PointerEnter { x, y },
                // Move before clicking: a motion to the position we just
                // entered at is legitimately coalesced away by the compositor.
                InputEvent::PointerMotion {
                    x: x + 5.0,
                    y: y + 5.0,
                },
                InputEvent::PointerButton {
                    button: 0x110, // BTN_LEFT
                    pressed: true,
                },
                InputEvent::PointerButton {
                    button: 0x110,
                    pressed: false,
                },
            ]
        }
    } else {
        let key = key.ok_or("pass --key or --click")?;
        vec![
            InputEvent::KeyboardEnter,
            InputEvent::KeyboardKey { key, pressed: true },
            InputEvent::KeyboardKey {
                key,
                pressed: false,
            },
        ]
    };

    for ev in events {
        let body = postcard::to_allocvec(&ev).map_err(|e| e.to_string())?;
        s.write_all(&(body.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        s.write_all(&body).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(60));
    }
    Ok(())
}
