use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    process::ExitCode,
};

use d2b_contracts::v3::ProviderManifest;
use d2b_provider_toolkit::manifest::{self, VerificationError};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os();
    let _program = args.next();
    let Some(namespace) = args.next() else {
        return Err(usage());
    };
    if namespace != "manifest" {
        return Err(usage());
    }

    let Some(operation) = args.next() else {
        return Err(usage());
    };
    match operation.to_string_lossy().as_ref() {
        "emit" => emit_command(&mut args),
        "verify" => verify_command(&mut args),
        _ => Err(usage()),
    }
}

fn emit_command(args: &mut impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let Some(flag) = args.next() else {
        return Err(usage());
    };
    if flag != "--out" {
        return Err(usage());
    }
    let Some(output) = args.next() else {
        return Err(usage());
    };
    if args.next().is_some() {
        return Err(usage());
    }

    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|_| "failed to read the manifest from stdin".to_owned())?;
    let manifest = serde_json::from_slice::<ProviderManifest>(&input)
        .map_err(|_| "failed to parse the Provider manifest from stdin".to_owned())?;
    let bytes = manifest::emit_canonical(&manifest);
    write_output(Path::new(&output), &bytes)
}

fn verify_command(args: &mut impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let Some(path) = args.next() else {
        return Err(usage());
    };
    if args.next().is_some() {
        return Err(usage());
    }
    let bytes = fs::read(&path).map_err(|_| "failed to read the manifest".to_owned())?;
    match manifest::verify_canonical(&bytes) {
        Ok(()) => {
            println!("canonical");
            Ok(())
        }
        Err(VerificationError::NotCanonical(mismatch)) => Err(format!(
            "provider-manifest-not-canonical: offset={} expected-len={} observed-len={}; \
             run d2b-provider-toolkit manifest emit --out <path>, then \
             d2b-provider-toolkit manifest verify <path>",
            mismatch.offset(),
            mismatch.expected_len(),
            mismatch.observed_len()
        )),
        Err(VerificationError::InvalidManifest) => {
            Err("failed to parse the Provider manifest".to_owned())
        }
    }
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|_| "failed to create the canonical manifest".to_owned())?;
    set_mode_0644(&file)?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|_| "failed to write the canonical manifest".to_owned())
}

#[cfg(unix)]
fn set_mode_0644(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(|_| "failed to set canonical manifest permissions".to_owned())
}

#[cfg(not(unix))]
fn set_mode_0644(_file: &std::fs::File) -> Result<(), String> {
    Ok(())
}

fn usage() -> String {
    "usage: d2b-provider-toolkit manifest emit --out <path> | \
     d2b-provider-toolkit manifest verify <path>"
        .to_owned()
}
