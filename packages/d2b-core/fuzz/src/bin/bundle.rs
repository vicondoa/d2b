#[path = "../harness.rs"]
mod harness;

use d2b_core::bundle::Bundle;

fn main() {
    let runs = harness::parse_runs(10000);
    harness::run_byte_fuzz("bundle", runs, |input| {
        let _ = serde_json::from_slice::<Bundle>(input);
    });
}
