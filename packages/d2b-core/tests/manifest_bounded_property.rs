use d2b_core::{
    bundle::Bundle, host::HostJson, manifest_v04::ManifestV04, privileges::PrivilegesJson,
};
use std::{fs, path::PathBuf};

const RUNS: usize = 10_000;
const MAX_LEN: usize = 4096;

#[test]
fn manifest_v04_bounded_byte_inputs_do_not_panic() {
    run_corpus("manifest_v04", |input| {
        let _ = ManifestV04::from_slice(input);
    });
    run_generated("manifest_v04", 0x4d41_4e49_4645_5354, |input| {
        let _ = ManifestV04::from_slice(input);
    });
}

#[test]
fn bundle_bounded_byte_inputs_do_not_panic() {
    run_corpus("bundle", |input| {
        let _ = serde_json::from_slice::<Bundle>(input);
    });
    run_generated("bundle", 0x4255_4e44_4c45_0001, |input| {
        let _ = serde_json::from_slice::<Bundle>(input);
    });
}

#[test]
fn host_json_bounded_byte_inputs_do_not_panic() {
    run_corpus("host", |input| {
        let _ = serde_json::from_slice::<HostJson>(input);
    });
    run_generated("host", 0x484f_5354_0000_0001, |input| {
        let _ = serde_json::from_slice::<HostJson>(input);
    });
}

#[test]
fn privileges_json_bounded_byte_inputs_do_not_panic() {
    run_corpus("privileges", |input| {
        let _ = serde_json::from_slice::<PrivilegesJson>(input);
    });
    run_generated("privileges", 0x5052_4956_494c_4547, |input| {
        let _ = serde_json::from_slice::<PrivilegesJson>(input);
    });
}

fn run_corpus<F>(target: &str, mut parser: F)
where
    F: FnMut(&[u8]),
{
    let corpus_dir = corpus_dir(target);
    let mut files: Vec<_> = fs::read_dir(&corpus_dir)
        .unwrap_or_else(|error| panic!("read corpus directory {corpus_dir:?}: {error}"))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "expected corpus files under {corpus_dir:?}"
    );

    for path in files {
        let bytes =
            fs::read(&path).unwrap_or_else(|error| panic!("read corpus file {path:?}: {error}"));
        parser(&bytes);
    }
}

fn run_generated<F>(target: &str, seed: u64, mut parser: F)
where
    F: FnMut(&[u8]),
{
    let mut rng = XorShift64::new(seed);
    for case in 0..RUNS {
        let input = generated_case(&mut rng, case);
        parser(&input);
    }
    eprintln!("bounded parser check completed {RUNS} generated {target} cases");
}

fn corpus_dir(target: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus")
        .join(target)
}

fn generated_case(rng: &mut XorShift64, case: usize) -> Vec<u8> {
    let len = match case % 16 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => (rng.next_u64() as usize) % (MAX_LEN + 1),
    };
    let mut bytes = Vec::with_capacity(len);
    for offset in 0..len {
        let raw = rng.next_u64() as u8;
        let byte = match (case + offset) % 13 {
            0 => b'{',
            1 => b'}',
            2 => b'[',
            3 => b']',
            4 => b'"',
            5 => b':',
            6 => b',',
            7 => b'-',
            8 => b'0' + (raw % 10),
            _ => raw,
        };
        bytes.push(byte);
    }
    bytes
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }
}
