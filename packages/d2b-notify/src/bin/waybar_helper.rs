// SPDX-License-Identifier: Apache-2.0
//! `d2b-sk-waybar-helper` — Waybar custom module helper for d2b security-key status.
//!
//! Reads one explicit, bounded security-key presentation projection and emits
//! one line of Waybar JSON to stdout, then exits. The projection is not an
//! endpoint, and the helper never falls back to a daemon or alternate path.
//!
//! Waybar `config` (excerpt):
//!
//! ```json
//! "custom/d2b-sk": {
//!     "exec": "d2b-sk-waybar-helper",
//!     "return-type": "json",
//!     "interval": 2
//! }
//! ```
//!
//! Exit codes:
//! - `0`: state read and block emitted (even if idle).
//! - `1`: projection absent or unreadable (Waybar hides the widget).
//! - `2`: invalid or unsupported projection (Waybar hides the widget).
//! - `64`: exactly one projection path was not supplied.

use d2b_notify::{
    state::{MAX_PROJECTION_BYTES, SkNotifyState},
    waybar::{print_waybar_block, waybar_block_from_state},
};
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    let path = match projection_path(std::env::args_os().skip(1)) {
        Ok(path) => path,
        Err(()) => {
            eprintln!("d2b-sk-waybar-helper: expected one projection path");
            std::process::exit(64);
        }
    };

    let mut state = match read_projection(&path) {
        Ok(state) => state,
        Err(ReadProjectionError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            std::process::exit(1);
        }
        Err(ReadProjectionError::Io(_)) => {
            eprintln!("d2b-sk-waybar-helper: projection unavailable");
            std::process::exit(1);
        }
        Err(ReadProjectionError::Invalid) => {
            eprintln!("d2b-sk-waybar-helper: invalid projection");
            std::process::exit(2);
        }
    };

    state.prune_stale_active(now_secs());

    let block = waybar_block_from_state(&state);
    if let Err(e) = print_waybar_block(&block) {
        eprintln!("d2b-sk-waybar-helper: output error: {e}");
        std::process::exit(2);
    }
}

fn projection_path(arguments: impl IntoIterator<Item = OsString>) -> Result<PathBuf, ()> {
    let mut arguments = arguments.into_iter();
    let path = arguments.next().ok_or(())?;
    if arguments.next().is_some() || path.is_empty() {
        return Err(());
    }
    Ok(PathBuf::from(path))
}

fn read_projection(path: &Path) -> Result<SkNotifyState, ReadProjectionError> {
    let file = std::fs::File::open(path).map_err(ReadProjectionError::Io)?;
    let mut bytes = Vec::with_capacity(MAX_PROJECTION_BYTES.min(4 * 1024));
    file.take((MAX_PROJECTION_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(ReadProjectionError::Io)?;
    SkNotifyState::from_slice(&bytes).map_err(|_| ReadProjectionError::Invalid)
}

#[derive(Debug)]
enum ReadProjectionError {
    Io(std::io::Error),
    Invalid,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_path_is_mandatory_and_has_no_fallback() {
        assert!(projection_path(Vec::<OsString>::new()).is_err());
        assert!(projection_path([OsString::from("one"), OsString::from("two")]).is_err());
        assert_eq!(
            projection_path([OsString::from("projection.json")]).unwrap(),
            PathBuf::from("projection.json")
        );
    }
}
