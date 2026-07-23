// SPDX-License-Identifier: Apache-2.0
//! `d2b-sk-waybar-helper` — Waybar custom module helper for d2b security-key status.
//!
//! Reads the durable security-key state file from the path given on the
//! command line (or the default `/run/d2b/notify/sk-state.json`) and emits
//! one line of Waybar JSON to stdout, then exits.
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
//! - `1`: state file absent or unreadable (Waybar hides the widget).
//! - `2`: JSON parse error or unsupported schema version (Waybar hides the widget).

use d2b_notify::{
    state::SkNotifyState,
    waybar::{print_waybar_block, waybar_block_from_state},
};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_STATE_PATH: &str = "/run/d2b/notify/sk-state.json";

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_STATE_PATH.to_owned());

    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("d2b-sk-waybar-helper: cannot read {path}: {e}");
            std::process::exit(1);
        }
    };

    let mut state = match SkNotifyState::from_json(&raw) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("d2b-sk-waybar-helper: {e}");
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
