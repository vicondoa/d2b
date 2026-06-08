#![allow(dead_code)]

#[cfg(feature = "fuzz")]
use std::fs;
use std::{env, path::PathBuf};

#[cfg(feature = "fuzz")]
pub fn parse_runs(default: usize) -> usize {
    let mut args = env::args().skip(1);
    let mut runs = default;

    while let Some(arg) = args.next() {
        if arg == "--runs" {
            if let Some(value) = args.next() {
                runs = value.parse().unwrap_or(default);
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--runs=") {
            runs = value.parse().unwrap_or(default);
        }
    }

    runs
}

pub fn repo_root() -> PathBuf {
    let core_root = nixling_core_root();
    core_root
        .parent()
        .expect("packages directory")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

pub fn nixling_core_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if manifest_dir.file_name().and_then(|name| name.to_str()) == Some("fuzz") {
        manifest_dir
            .parent()
            .expect("nixling-core parent directory")
            .to_path_buf()
    } else {
        manifest_dir
    }
}

#[cfg(feature = "fuzz")]
pub fn corpus_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let direct = manifest_dir.join("corpus");
    if direct.is_dir() {
        direct
    } else {
        manifest_dir.join("fuzz/corpus")
    }
}

pub fn run_named_tests(tests: &[(&str, fn())]) {
    let filter = first_filter_arg();
    let mut failures = 0usize;
    let mut executed = 0usize;

    for (name, test) in tests {
        if filter.as_ref().is_some_and(|value| !name.contains(value)) {
            continue;
        }
        executed += 1;
        eprint!("test {name} ... ");
        match std::panic::catch_unwind(*test) {
            Ok(()) => eprintln!("ok"),
            Err(payload) => {
                failures += 1;
                eprintln!("FAILED");
                eprintln!("  {}", panic_message(&payload));
            }
        }
    }

    if executed == 0 {
        eprintln!("no tests matched filter");
        std::process::exit(1);
    }
    if failures > 0 {
        std::process::exit(1);
    }
}

#[cfg(feature = "fuzz")]
pub fn run_corpus<F>(target: &str, mut parser: F)
where
    F: FnMut(&[u8]),
{
    let corpus_dir = corpus_root().join(target);
    let mut files: Vec<_> = fs::read_dir(&corpus_dir)
        .unwrap_or_else(|error| panic!("failed to read corpus directory {corpus_dir:?}: {error}"))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();

    assert!(
        !files.is_empty(),
        "expected committed corpus files under {corpus_dir:?}"
    );

    for path in files {
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read corpus file {path:?}: {error}"));
        parser(&bytes);
    }
}

#[cfg(feature = "fuzz")]
pub fn run_byte_fuzz<F>(target: &str, runs: usize, mut parser: F)
where
    F: FnMut(&[u8]) + std::panic::RefUnwindSafe,
{
    run_corpus(target, |bytes| parser(bytes));
    bolero::check!(name = target)
        .with_iterations(runs)
        .with_max_len(4096)
        .for_each(|input: &[u8]| {
            parser(input);
        });
}

fn first_filter_arg() -> Option<String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--runs" {
            let _ = args.next();
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        return Some(arg);
    }
    None
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "test panicked without a string payload".to_owned()
    }
}
