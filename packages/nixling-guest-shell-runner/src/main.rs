use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use nixling_guest_shell_runner::{
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
            bail!(
                "persistent shell daemon mode is not enabled in this helper build: home={}",
                args.home.display()
            );
        }
        Command::Attach(args) => {
            validate_socket_path(&args.socket)?;
            validate_shell_name(&args.name)?;
            bail!(
                "persistent shell attach mode is not enabled in this helper build: force={}",
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
                println!("shell session listing is not implemented in this helper build");
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

fn print_management(command: &'static str, name: String, json: bool) -> Result<()> {
    let output = ShellManagementOutput::unsupported(command, name);
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "{} for '{}' is not implemented in this helper build",
            output.command, output.name
        );
    }
    std::io::Write::flush(&mut std::io::stdout()).context("flushing stdout")
}
