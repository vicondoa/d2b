use nixling_core::manifest_v04::ManifestV04;
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: cargo run -p xtask --bin manifest_v04_check -- <input> [output]");
        return ExitCode::FAILURE;
    };
    let output = args.next();
    if args.next().is_some() {
        eprintln!("usage: cargo run -p xtask --bin manifest_v04_check -- <input> [output]");
        return ExitCode::FAILURE;
    }

    let manifest = match ManifestV04::from_path(std::path::Path::new(&input)) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("manifest_v04_check: {error}");
            return ExitCode::FAILURE;
        }
    };
    let rendered = match manifest.to_compact_json() {
        Ok(rendered) => rendered,
        Err(error) => {
            eprintln!("manifest_v04_check: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(output) = output {
        if let Err(error) = fs::write(&output, rendered) {
            eprintln!("manifest_v04_check: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        print!("{rendered}");
    }

    ExitCode::SUCCESS
}
