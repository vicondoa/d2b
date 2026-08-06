#![allow(dead_code)]

use std::ffi::OsString;

use clap::CommandFactory;

mod activation;
mod complete;
mod context;
mod dispatch;
mod doctor;
mod endpoint;
mod exec;
mod exec_client;
mod guest;
mod host;
mod host_validate;
mod legacy;
mod provider;
mod resource;
mod share;
mod shell;
mod status_read_model;
mod target_routing;
mod terminal_client;
mod zone;
mod zone_audit;
mod zone_doctor;
mod zone_support_bundle;

#[allow(unused_imports)]
use legacy::*;

const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Exit code for api-ready timeout in strict mode.
pub const EXIT_API_TIMEOUT: i32 = 33;

pub fn cli_command() -> clap::Command {
    let mut command = dispatch::ModernCli::command();
    command.set_bin_name("d2b");
    command
}

pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let raw_args: Vec<OsString> = args.into_iter().collect();
    if raw_args.is_empty() {
        return 1;
    }

    if raw_args.len() == 1 {
        print_stdout("d2b 0.0.0-bootstrap\n");
        print_stdout("Run `d2b --help` for the typed Zone command surface.\n");
        return 0;
    }

    dispatch::modern_run(raw_args)
}
