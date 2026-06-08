use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(nixling::run(std::env::args_os()) as u8)
}
