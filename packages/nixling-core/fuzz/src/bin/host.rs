#[path = "../harness.rs"]
mod harness;

use nixling_core::host::HostJson;

fn main() {
    let runs = harness::parse_runs(10000);
    harness::run_byte_fuzz("host", runs, |input| {
        let _ = serde_json::from_slice::<HostJson>(input);
    });
}
