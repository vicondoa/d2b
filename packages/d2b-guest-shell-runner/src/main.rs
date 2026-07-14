use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use d2b_guest_shell_runner::{
    cli::{Cli, Command},
    name::validate_shell_name,
    output::ShellManagementOutput,
    socket::validate_socket_path,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Daemon(args) => {
            validate_socket_path(&args.socket)?;
            run_libshpool_daemon(&args.socket, &args.home)?;
        }
        Command::Attach(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            run_libshpool_attach(&args.socket, &args.name, args.force)?;
        }
        Command::List(args) => {
            validate_socket_path(&args.socket)?;
            run_libshpool_list(&args.socket, args.json)?;
        }
        Command::Detach(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            let status = run_libshpool_session_command("detach", &args.socket, &args.name)?;
            print_management("detach", args.name, args.json, status)?;
        }
        Command::Kill(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            let status = run_libshpool_session_command("kill", &args.socket, &args.name)?;
            print_management("kill", args.name, args.json, status)?;
        }
    }
    Ok(())
}

#[cfg(feature = "real-libshpool")]
fn parse_shpool_args(argv: Vec<String>) -> Result<libshpool::Args> {
    libshpool::Args::try_parse_from(argv).context("parsing shpool arguments")
}

#[cfg(feature = "real-libshpool")]
fn run_shpool(argv: Vec<String>) -> Result<()> {
    let args = parse_shpool_args(argv)?;
    if args.version() {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    d2b_guest_shell_runner::libshpool_bridge::run(args)
}

#[cfg(feature = "real-libshpool")]
fn socket_arg(socket: &Path) -> String {
    socket.to_string_lossy().into_owned()
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_daemon(socket: &Path, home: &Path) -> Result<()> {
    let args = parse_shpool_args(vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        "daemon".to_owned(),
    ])?;
    if args.version() {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    d2b_guest_shell_runner::libshpool_bridge::run_with_home(args, home)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_daemon(_socket: &Path, _home: &Path) -> Result<()> {
    anyhow::bail!("persistent shell daemon mode is not enabled in this helper build")
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_attach(socket: &Path, name: &str, force: bool) -> Result<()> {
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
    argv.push("--".to_owned());
    argv.push(name.to_owned());
    run_shpool(argv)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_attach(_socket: &Path, _name: &str, _force: bool) -> Result<()> {
    anyhow::bail!("persistent shell attach mode is not enabled in this helper build")
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_list(socket: &Path, json: bool) -> Result<()> {
    let mut argv = vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        "list".to_owned(),
    ];
    if json {
        argv.push("--json".to_owned());
    }
    run_shpool(argv)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_list(_socket: &Path, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string(&ShellManagementOutput::unsupported("list", String::new()))?
        );
    } else {
        println!("shell session listing is not implemented in this helper build");
    }
    Ok(())
}

#[cfg(feature = "real-libshpool")]
fn run_libshpool_session_command(
    command: &str,
    socket: &Path,
    name: &str,
) -> Result<ManagementStatus> {
    run_shpool(vec![
        "shpool".to_owned(),
        "--socket".to_owned(),
        socket_arg(socket),
        "--no-daemonize".to_owned(),
        command.to_owned(),
        "--".to_owned(),
        name.to_owned(),
    ])?;
    Ok(ManagementStatus::Ok)
}

#[cfg(not(feature = "real-libshpool"))]
fn run_libshpool_session_command(
    _command: &str,
    _socket: &Path,
    _name: &str,
) -> Result<ManagementStatus> {
    Ok(ManagementStatus::Unsupported)
}

#[derive(Clone, Copy)]
enum ManagementStatus {
    #[cfg_attr(not(feature = "real-libshpool"), allow(dead_code))]
    Ok,
    #[cfg(not(feature = "real-libshpool"))]
    Unsupported,
}

fn print_management(
    command: &'static str,
    name: String,
    json: bool,
    status: ManagementStatus,
) -> Result<()> {
    let output = match status {
        ManagementStatus::Ok => ShellManagementOutput::ok(command, name),
        #[cfg(not(feature = "real-libshpool"))]
        ManagementStatus::Unsupported => ShellManagementOutput::unsupported(command, name),
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        match status {
            ManagementStatus::Ok => println!("{} for '{}' completed", output.command, output.name),
            #[cfg(not(feature = "real-libshpool"))]
            ManagementStatus::Unsupported => println!(
                "{} for '{}' is not implemented in this helper build",
                output.command, output.name
            ),
        }
    }
    std::io::Write::flush(&mut std::io::stdout()).context("flushing stdout")
}
