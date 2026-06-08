#[path = "../harness.rs"]
mod harness;

use nixling_core::privileges::PrivilegesJson;

fn main() {
    let runs = harness::parse_runs(10000);
    harness::run_byte_fuzz("privileges", runs, |input| {
        let _ = serde_json::from_slice::<PrivilegesJson>(input);
    });
}
