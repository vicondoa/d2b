use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(d2b::run(std::env::args_os()) as u8)
}
