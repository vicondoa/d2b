use std::path::Path;

use anyhow::{Result, anyhow};
use clap::Parser;

use crate::socket::ExternalDataPlaneSocket;

fn parse(argv: Vec<String>) -> Result<libshpool::Args> {
    libshpool::Args::try_parse_from(argv)
        .map_err(|_| anyhow!("libshpool-backend-arguments-invalid"))
}

#[allow(unsafe_code)]
fn run(args: libshpool::Args) -> Result<()> {
    // libshpool 0.11.0 documents `run` as unsafe because it can initialize
    // global tracing state, daemonize, and exit the process. The helper is the
    // process boundary that contains those effects; guestd never calls this.
    unsafe { libshpool::run(args, None) }.map_err(|_| anyhow!("libshpool-backend-failed"))
}

#[allow(unsafe_code)]
fn run_with_home(args: libshpool::Args, home: &Path) -> Result<()> {
    // The daemon helper is single-threaded before this call and exits by running
    // libshpool; mutating HOME here is the narrow process-boundary effect the
    // helper exists to contain.
    unsafe {
        std::env::set_var("HOME", home);
    }
    run(args)
}

fn socket_arg(socket: &ExternalDataPlaneSocket) -> String {
    socket.as_path().to_string_lossy().into_owned()
}

pub fn daemon(socket: &ExternalDataPlaneSocket, home: &Path) -> Result<()> {
    let args = parse(vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        "daemon".to_owned(),
    ])?;
    run_with_home(args, home)
}

pub fn attach(socket: &ExternalDataPlaneSocket, name: &str, force: bool) -> Result<()> {
    let mut argv = vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        "attach".to_owned(),
    ];
    if force {
        argv.push("--force".to_owned());
    }
    argv.extend(["--".to_owned(), name.to_owned()]);
    run(parse(argv)?)
}

pub fn list(socket: &ExternalDataPlaneSocket) -> Result<()> {
    run(parse(vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        "list".to_owned(),
    ])?)
}

fn session_command(
    command: &'static str,
    socket: &ExternalDataPlaneSocket,
    name: &str,
) -> Result<()> {
    run(parse(vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        command.to_owned(),
        "--".to_owned(),
        name.to_owned(),
    ])?)
}

pub fn detach(socket: &ExternalDataPlaneSocket, name: &str) -> Result<()> {
    session_command("detach", socket, name)
}

pub fn kill(socket: &ExternalDataPlaneSocket, name: &str) -> Result<()> {
    session_command("kill", socket, name)
}
