use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use redb_resource_store_spike::{
    crash_database_path, prepare_crash_database, run_crash_transaction, verify_crash_database,
};

fn parse_boundary(arguments: &[String]) -> Result<u8, String> {
    let position = arguments
        .iter()
        .position(|argument| argument == "--kill-at-txn")
        .ok_or_else(|| "usage: crash-fixture --kill-at-txn <1..13>".to_owned())?;
    arguments
        .get(position + 1)
        .ok_or_else(|| "missing transaction boundary".to_owned())?
        .parse::<u8>()
        .map_err(|error| format!("invalid transaction boundary: {error}"))
}

fn parse_database(arguments: &[String]) -> Result<std::path::PathBuf, String> {
    let position = arguments
        .iter()
        .position(|argument| argument == "--database")
        .ok_or_else(|| "missing worker database path".to_owned())?;
    arguments
        .get(position + 1)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "missing worker database path value".to_owned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let boundary = parse_boundary(&arguments)?;
    if arguments.iter().any(|argument| argument == "--worker") {
        let path = parse_database(&arguments)?;
        run_crash_transaction(&path, boundary)?;
        return Err("worker was not terminated by SIGKILL".into());
    }

    let path = crash_database_path(boundary);
    let checkpoints = prepare_crash_database(&path)?;
    let status = Command::new(std::env::current_exe()?)
        .arg("--worker")
        .arg("--kill-at-txn")
        .arg(boundary.to_string())
        .arg("--database")
        .arg(&path)
        .status()?;
    if status.signal() != Some(9) {
        return Err(format!("worker exit was not SIGKILL: {status}").into());
    }
    let recovery = verify_crash_database(&path, &checkpoints)?;
    println!("boundary={boundary} worker_signal=9 recovery={recovery:?} result=PASS");
    std::fs::remove_file(path)?;
    Ok(())
}
