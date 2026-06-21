use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use nixling_guest_shell_runner::{
    cli::{Cli, Command},
    name::validate_shell_name,
    output::ShellManagementOutput,
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
            bail!(
                "real shpool daemon mode is gated behind later Wave 0 runtime plumbing: {}",
                args.home.display()
            );
        }
        Command::Attach(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            bail!(
                "real shpool attach mode is gated behind later Wave 0 runtime plumbing: force={}",
                args.force
            );
        }
        Command::List(args) => {
            validate_socket_path(&args.socket)?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string(&ShellManagementOutput::unsupported(
                        "list",
                        String::new()
                    ))?
                );
            } else {
                println!("shell session listing is not wired yet");
            }
        }
        Command::Detach(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            print_management("detach", args.name, args.json)?;
        }
        Command::Kill(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            print_management("kill", args.name, args.json)?;
        }
    }
    Ok(())
}

fn validate_socket_path(path: &std::path::Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("socket path must not be empty");
    }
    if !path.is_absolute() {
        bail!("socket path must be absolute: {}", path.display());
    }
    if path.as_os_str().as_encoded_bytes().len() >= 108 {
        bail!(
            "socket path is too long for sockaddr_un: {}",
            path.display()
        );
    }
    Ok(())
}

fn print_management(command: &'static str, name: String, json: bool) -> Result<()> {
    let output = ShellManagementOutput::unsupported(command, name);
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{} for '{}' is not wired yet", output.command, output.name);
    }
    std::io::Write::flush(&mut std::io::stdout()).context("flushing stdout")
}
