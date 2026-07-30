//! Command-line surface.
//!
//! `run` starts a session. Every other subcommand is a one-shot client that
//! connects to a running session's socket, sends one request and prints one
//! response. That shape is deliberate: an agent drives this with ordinary shell
//! commands and never has to hold a stdio session open.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::protocol::{Request, Response, encode_line};
use crate::pump::{self, PumpConfig};
use crate::server::default_socket_path;
use crate::session::DEFAULT_SCROLLBACK;
use crate::tty::TtySize;

#[derive(Debug, Parser)]
#[command(
    name = "d2b-agentterm",
    about = "Run a terminal program that both a human and an agent can drive",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a program, passing the terminal through and serving an agent socket.
    Run(RunArgs),
    /// Print session metadata.
    Info(ClientArgs),
    /// Print the current screen.
    Screen(ScreenArgs),
    /// Print what changed over a trailing window.
    Delta(DeltaArgs),
    /// Send key names, e.g. `Enter`, `Down`, `C-c`.
    Keys(KeysArgs),
    /// Send literal text.
    Text(TextArgs),
    /// Send raw bytes as an escape hatch.
    Raw(RawArgs),
    /// Request a resize. Advisory; the attached terminal wins.
    Resize(ResizeArgs),
    /// Print a sequence that reconstructs the current screen.
    Dump(ClientArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Initial size as COLSxROWS. Defaults to the attached terminal's size.
    #[arg(long)]
    pub size: Option<String>,

    /// Lines of scrollback the emulator retains.
    #[arg(long, default_value_t = DEFAULT_SCROLLBACK)]
    pub scrollback: usize,

    /// Socket path. Defaults to $XDG_RUNTIME_DIR/d2b-agentterm-<pid>.sock.
    #[arg(long)]
    pub socket: Option<PathBuf>,

    /// Suppress the startup banner on stderr.
    #[arg(long)]
    pub quiet: bool,

    /// The program to run, after `--`.
    #[arg(trailing_var_arg = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Args, Clone)]
pub struct ClientArgs {
    /// Socket path of the running session.
    #[arg(long)]
    pub socket: Option<PathBuf>,

    /// Emit JSON rather than human-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ScreenArgs {
    #[command(flatten)]
    pub client: ClientArgs,
}

#[derive(Debug, Args)]
pub struct DeltaArgs {
    #[command(flatten)]
    pub client: ClientArgs,

    /// Window to look back over, e.g. `10s`, `500ms`, `2m`.
    #[arg(long, default_value = "10s")]
    pub since: String,
}

#[derive(Debug, Args)]
pub struct KeysArgs {
    #[command(flatten)]
    pub client: ClientArgs,

    /// Key names to send.
    #[arg(required = true)]
    pub keys: Vec<String>,
}

#[derive(Debug, Args)]
pub struct TextArgs {
    #[command(flatten)]
    pub client: ClientArgs,

    /// Text to send.
    #[arg(required = true)]
    pub text: String,
}

#[derive(Debug, Args)]
pub struct RawArgs {
    #[command(flatten)]
    pub client: ClientArgs,

    /// Bytes to send, interpreted as UTF-8.
    #[arg(required = true)]
    pub data: String,
}

#[derive(Debug, Args)]
pub struct ResizeArgs {
    #[command(flatten)]
    pub client: ClientArgs,

    #[arg(long)]
    pub cols: u16,

    #[arg(long)]
    pub rows: u16,
}

/// Parse `COLSxROWS`.
pub fn parse_size(text: &str) -> anyhow::Result<TtySize> {
    let (cols, rows) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("size must be COLSxROWS, got {text:?}"))?;

    let cols: u16 = cols
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid column count in {text:?}"))?;
    let rows: u16 = rows
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid row count in {text:?}"))?;

    if cols == 0 || rows == 0 {
        anyhow::bail!("size dimensions must be non-zero, got {text:?}");
    }

    Ok(TtySize::new(cols, rows))
}

/// Parse a duration such as `10s`, `500ms`, `2m`, or a bare number of seconds.
pub fn parse_duration(text: &str) -> anyhow::Result<Duration> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("empty duration");
    }

    let (value, unit) = match text.strip_suffix("ms") {
        Some(value) => (value, "ms"),
        None => match text.strip_suffix('s') {
            Some(value) => (value, "s"),
            None => match text.strip_suffix('m') {
                Some(value) => (value, "m"),
                None => (text, "s"),
            },
        },
    };

    let number: f64 = value
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration {text:?}"))?;

    if !number.is_finite() || number < 0.0 {
        anyhow::bail!("duration must be finite and non-negative, got {text:?}");
    }

    let millis = match unit {
        "ms" => number,
        "s" => number * 1000.0,
        "m" => number * 60_000.0,
        _ => number * 1000.0,
    };

    Ok(Duration::from_millis(millis as u64))
}

/// Resolve the socket path, defaulting to the sole running session if there is
/// exactly one.
fn resolve_socket(explicit: &Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.clone());
    }

    if let Some(path) = std::env::var_os("D2B_AGENTTERM_SOCKET") {
        return Ok(PathBuf::from(path));
    }

    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    let mut found: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("d2b-agentterm-") && name.ends_with(".sock") {
                found.push(entry.path());
            }
        }
    }

    found.sort();

    match found.len() {
        0 => anyhow::bail!(
            "no running session found in {}; pass --socket",
            dir.display()
        ),
        1 => Ok(found.remove(0)),
        n => anyhow::bail!(
            "{n} sessions running in {}; pass --socket to choose one",
            dir.display()
        ),
    }
}

/// Send one request and return the response.
pub async fn request(socket: &PathBuf, request: Request) -> anyhow::Result<Response> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|err| anyhow::anyhow!("cannot connect to {}: {err}", socket.display()))?;

    let (read_half, mut write_half) = stream.into_split();
    write_half
        .write_all(encode_line(&request)?.as_bytes())
        .await?;
    write_half.flush().await?;

    let mut lines = BufReader::new(read_half).lines();
    match lines.next_line().await? {
        Some(line) => Ok(serde_json::from_str(&line)?),
        None => anyhow::bail!("session closed the connection without responding"),
    }
}

/// Entry point.
pub async fn main(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Command::Run(args) => run(args).await,
        Command::Info(args) => client(args, Request::Info).await,
        Command::Screen(args) => client(args.client, Request::Screen).await,
        Command::Delta(args) => {
            let window = parse_duration(&args.since)?;
            client(
                args.client,
                Request::Delta {
                    window_ms: window.as_millis() as u64,
                },
            )
            .await
        }
        Command::Keys(args) => client(args.client, Request::Keys { keys: args.keys }).await,
        Command::Text(args) => client(args.client, Request::Text { text: args.text }).await,
        Command::Raw(args) => client(args.client, Request::Raw { data: args.data }).await,
        Command::Resize(args) => {
            client(
                args.client,
                Request::Resize {
                    cols: args.cols,
                    rows: args.rows,
                },
            )
            .await
        }
        Command::Dump(args) => client(args, Request::Dump).await,
    }
}

async fn run(args: RunArgs) -> anyhow::Result<i32> {
    let size = match &args.size {
        Some(text) => Some(parse_size(text)?),
        None => None,
    };

    let socket = args
        .socket
        .unwrap_or_else(|| default_socket_path(std::process::id()));

    pump::run(PumpConfig {
        command: args.command,
        size,
        scrollback: args.scrollback,
        socket,
        announce: !args.quiet,
    })
    .await
}

async fn client(args: ClientArgs, req: Request) -> anyhow::Result<i32> {
    let socket = resolve_socket(&args.socket)?;
    let response = request(&socket, req).await?;
    let is_error = response.is_error();

    let rendered = if args.json {
        serde_json::to_string_pretty(&response)?
    } else {
        render_human(&response)
    };

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{rendered}")?;

    Ok(if is_error { 1 } else { 0 })
}

fn render_human(response: &Response) -> String {
    match response {
        Response::Info(info) => {
            let alt = if info.alt_screen { "alt" } else { "primary" };
            format!(
                "pid {}  size {}x{}  buffer {}  paste {}  appcursor {}  up {}ms{}",
                info.child_pid,
                info.cols,
                info.rows,
                alt,
                info.bracketed_paste,
                info.cursor_key_app_mode,
                info.uptime_ms,
                match info.exit_status {
                    Some(code) => format!("  exited {code}"),
                    None => String::new(),
                }
            )
        }
        Response::Screen(snap) => snap.view_text(),
        Response::Delta(report) => report.render_human(),
        Response::Applied(applied) => {
            let mut out = format!(
                "sent {} bytes (bracketed {})  size {}x{}",
                applied.bytes, applied.bracketed, applied.cols, applied.rows
            );
            if let Some(note) = &applied.note {
                out.push_str(&format!("\nnote: {note}"));
            }
            out
        }
        Response::Dump { seq } => seq.clone(),
        Response::Error { message } => format!("error: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_duration, parse_size};
    use std::time::Duration;

    #[test]
    fn size_parses_the_standard_form() {
        let size = parse_size("120x40").unwrap_or(crate::tty::TtySize::new(0, 0));
        assert_eq!(size.cols, 120);
        assert_eq!(size.rows, 40);
    }

    #[test]
    fn size_accepts_uppercase_separator() {
        assert!(parse_size("80X24").is_ok());
    }

    #[test]
    fn size_rejects_malformed_input() {
        assert!(parse_size("120").is_err());
        assert!(parse_size("axb").is_err());
        assert!(parse_size("").is_err());
    }

    #[test]
    fn size_rejects_zero_dimensions() {
        // A zero dimension makes the emulator panic on construction, so it has
        // to be refused at the boundary.
        assert!(parse_size("0x40").is_err());
        assert!(parse_size("120x0").is_err());
    }

    #[test]
    fn duration_parses_units() {
        assert_eq!(parse_duration("10s").ok(), Some(Duration::from_secs(10)));
        assert_eq!(
            parse_duration("500ms").ok(),
            Some(Duration::from_millis(500))
        );
        assert_eq!(parse_duration("2m").ok(), Some(Duration::from_secs(120)));
    }

    #[test]
    fn duration_defaults_to_seconds() {
        assert_eq!(parse_duration("30").ok(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn duration_accepts_fractions() {
        assert_eq!(
            parse_duration("1.5s").ok(),
            Some(Duration::from_millis(1500))
        );
    }

    #[test]
    fn duration_rejects_nonsense() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("-5s").is_err());
    }

    #[test]
    fn ms_suffix_is_matched_before_s_suffix() {
        // "500ms" ends with 's' too; checking the longer suffix first is what
        // stops this being parsed as 500 million milliseconds.
        assert_eq!(
            parse_duration("500ms").ok(),
            Some(Duration::from_millis(500))
        );
    }
}
