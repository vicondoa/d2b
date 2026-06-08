#[path = "../harness.rs"]
mod harness;

use nixling_core::manifest_v04::ManifestV04;

fn main() {
    let runs = harness::parse_runs(10000);
    harness::run_byte_fuzz("manifest_v04", runs, |input| {
        let _ = ManifestV04::from_slice(input);
    });
}
