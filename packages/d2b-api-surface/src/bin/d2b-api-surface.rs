use std::{env, path::PathBuf, process::ExitCode};

use d2b_api_surface::{
    PolicyError, Result, Snapshots, analyze, check_snapshot, error_label, load_root_spec,
    load_workspace, operation, write_snapshot,
};

struct Args {
    public_json_dir: PathBuf,
    private_json_dir: PathBuf,
    metadata: PathBuf,
    roots: PathBuf,
    public_api: PathBuf,
    capability_api: PathBuf,
    hidden_public_api: PathBuf,
    trait_impls: PathBuf,
    mode: Mode,
}

#[derive(Clone, Copy)]
enum Mode {
    Check,
    Write,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args(env::args_os().skip(1))?;
    let workspace = load_workspace(
        &args.public_json_dir,
        &args.private_json_dir,
        &args.metadata,
    )?;
    let roots = load_root_spec(&args.roots)?;
    let snapshots = analyze(&workspace, &roots)?;
    apply_snapshots(&args, &snapshots)
}

fn parse_args(arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<Args> {
    let arguments = arguments.collect::<Vec<_>>();
    let mut values = std::collections::BTreeMap::new();
    let mut mode = None;
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].to_str().ok_or_else(argument_error)?;
        match flag {
            "--check" => set_mode(&mut mode, Mode::Check)?,
            "--write" => set_mode(&mut mode, Mode::Write)?,
            "--public-json-dir"
            | "--private-json-dir"
            | "--metadata"
            | "--roots"
            | "--public-api"
            | "--capability-api"
            | "--hidden-public-api"
            | "--trait-impls" => {
                index += 1;
                let value = arguments.get(index).ok_or_else(argument_error)?;
                if values
                    .insert(flag.to_owned(), PathBuf::from(value))
                    .is_some()
                {
                    return Err(argument_error());
                }
            }
            _ => return Err(argument_error()),
        }
        index += 1;
    }
    Ok(Args {
        public_json_dir: take(&mut values, "--public-json-dir")?,
        private_json_dir: take(&mut values, "--private-json-dir")?,
        metadata: take(&mut values, "--metadata")?,
        roots: take(&mut values, "--roots")?,
        public_api: take(&mut values, "--public-api")?,
        capability_api: take(&mut values, "--capability-api")?,
        hidden_public_api: take(&mut values, "--hidden-public-api")?,
        trait_impls: take(&mut values, "--trait-impls")?,
        mode: mode.ok_or_else(argument_error)?,
    })
}

fn take(values: &mut std::collections::BTreeMap<String, PathBuf>, flag: &str) -> Result<PathBuf> {
    values.remove(flag).ok_or_else(argument_error)
}

fn set_mode(mode: &mut Option<Mode>, value: Mode) -> Result<()> {
    if mode.replace(value).is_some() {
        return Err(argument_error());
    }
    Ok(())
}

fn argument_error() -> PolicyError {
    PolicyError::new(operation::ARGUMENT_PARSE, error_label::INVALID_ARGUMENTS)
}

fn apply_snapshots(args: &Args, snapshots: &Snapshots) -> Result<()> {
    match args.mode {
        Mode::Check => {
            check_named_snapshot("public-api", &args.public_api, &snapshots.public_api)?;
            check_named_snapshot(
                "capability-api",
                &args.capability_api,
                &snapshots.capability_api,
            )?;
            check_named_snapshot(
                "hidden-public-api",
                &args.hidden_public_api,
                &snapshots.hidden_public_api,
            )?;
            check_named_snapshot(
                "capability-trait-impls",
                &args.trait_impls,
                &snapshots.capability_trait_impls,
            )?;
        }
        Mode::Write => {
            write_snapshot(&args.public_api, &snapshots.public_api)?;
            write_snapshot(&args.capability_api, &snapshots.capability_api)?;
            write_snapshot(&args.hidden_public_api, &snapshots.hidden_public_api)?;
            write_snapshot(&args.trait_impls, &snapshots.capability_trait_impls)?;
        }
    }
    Ok(())
}

fn check_named_snapshot(name: &str, path: &std::path::Path, lines: &[String]) -> Result<()> {
    check_snapshot(path, lines).inspect_err(|_error| {
        eprintln!("snapshot {name} failed; regenerate with `make api-surface-pin`");
    })
}
