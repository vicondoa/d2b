//! Command entry point for the resource compiler package.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "d2b-resource-compiler is library-driven; invoke the resource compiler through the \
         build integration"
    );
    ExitCode::from(2)
}
