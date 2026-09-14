//! Blocking-API census (U32, issue #524).
//!
//! Counts the workspace's uses of every API on the `clippy.toml`
//! `disallowed-methods` deny list, separating production from test contexts.
//! The deny list is the single source of truth - whatever it names is what
//! this counts - so the two cannot drift, and the production total is the
//! number U32 drives toward zero. The lint is the gate; this is the meter.

use std::fs;
use std::path::Path;

/// One entry from the deny list: the fully-qualified API path, and the bare
/// tail clippy matches on (everything after the final `::`).
pub struct DeniedApi {
    pub path: String,
    pub tail: String,
}

/// Parse the `path = "..."` entries of a clippy.toml `disallowed-methods` list.
pub fn parse_deny_list(clippy_toml: &str) -> Vec<DeniedApi> {
    let mut entries = Vec::new();
    for line in clippy_toml.lines() {
        let Some(start) = line.find("path = \"") else {
            continue;
        };
        let rest = &line[start + "path = \"".len()..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let path = rest[..end].to_owned();
        // clippy resolves the configured path to a method; the nearest
        // distinctive text form is the last two segments (`socket::connect`
        // rather than a bare `connect`, which collides with every adapter's
        // async connect). A path with one segment keeps that segment.
        let segments: Vec<&str> = path.split("::").collect();
        let tail = if segments.len() >= 2 {
            format!("{}::{}", segments[segments.len() - 2], segments[segments.len() - 1])
        } else {
            path.clone()
        };
        entries.push(DeniedApi { path, tail });
    }
    entries
}

/// One context's lines: `(line_number, trimmed_text)`.
pub type ContextLines = Vec<(usize, String)>;

/// A source file split into its production and test contexts.
pub type SplitContext = (ContextLines, ContextLines);

/// Split a source file's lines into production and test contexts.
///
/// A file is test context wholesale when its path lives in a `tests/`,
/// `integration/`, or `benches/` directory. Otherwise, lines inside a
/// `#[cfg(test)]` block are test context; everything else is production.
pub fn split_contexts(raw: &str, path_is_test: bool) -> SplitContext {
    let mut production = Vec::new();
    let mut test = Vec::new();
    let mut cfg_test_depth: Option<isize> = None;
    let mut marker_line_pending = false;

    for (index, line) in raw.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();

        if cfg_test_depth.is_none() && trimmed.contains("#[cfg(test)]") {
            cfg_test_depth = Some(0);
            marker_line_pending = true;
        }

        let depth_here = trimmed.chars().filter(|c| *c == '{').count() as isize
            - trimmed.chars().filter(|c| *c == '}').count() as isize;

        if let Some(depth) = &mut cfg_test_depth {
            *depth += depth_here;
        }

        let is_test = path_is_test || cfg_test_depth.is_some();
        if is_test {
            test.push((line_number, trimmed.to_owned()));
        } else {
            production.push((line_number, trimmed.to_owned()));
        }

        if let Some(depth) = cfg_test_depth
            && depth <= 0
            && !marker_line_pending
        {
            cfg_test_depth = None;
        }
        marker_line_pending = false;
    }
    (production, test)
}

/// Count each denied API's occurrences per context, with the first few
/// production locations for the report.
pub fn count_occurrences(
    files: &[(String, SplitContext)],
    entries: &[DeniedApi],
) -> Vec<(usize, usize, usize, Vec<String>)> {
    entries
        .iter()
        .map(|entry| {
            let mut production_hits = 0;
            let mut test_hits = 0;
            let mut samples = Vec::new();
            for (path, (production, test)) in files {
                for (line, text) in production {
                    if text.contains(&entry.tail) {
                        production_hits += 1;
                        if samples.len() < 5 {
                            samples.push(format!("{}:{}", path, line));
                        }
                    }
                }
                for (_, text) in test {
                    if text.contains(&entry.tail) {
                        test_hits += 1;
                    }
                }
            }
            (production_hits, test_hits, 0, samples)
        })
        .collect()
}

fn is_test_dir(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == "tests" || name == "integration" || name == "benches"
    })
}

/// Run the census over the repository. Prints the per-class table and the
/// production total; the totals are the measurement U32 drives down.
pub fn run(repo_root: &Path) -> Result<(), String> {
    let clippy_toml = fs::read_to_string(repo_root.join("clippy.toml"))
        .map_err(|_| "blocking-census: clippy.toml unreadable".to_owned())?;
    let entries = parse_deny_list(&clippy_toml);
    if entries.is_empty() {
        return Err("blocking-census: no entries parsed from clippy.toml".to_owned());
    }

    let mut files = Vec::new();
    for entry in walk_rs(repo_root.join("packages"))? {
        let path_is_test = is_test_dir(&entry);
        let raw = fs::read_to_string(&entry)
            .map_err(|error| format!("blocking-census: read {}: {error}", entry.display()))?;
        files.push((entry.display().to_string(), split_contexts(&raw, path_is_test)));
    }

    let counts = count_occurrences(&files, &entries);
    println!("{:72} {:>4} {:>4}", "denied API (clippy.toml path)", "prod", "test");
    let mut production_total = 0;
    let mut test_total = 0;
    let mut reported = 0;
    for (entry, count) in entries.iter().zip(&counts) {
        let (production_hits, test_hits, _, samples) = count;
        production_total += production_hits;
        test_total += test_hits;
        if *production_hits > 0 || *test_hits > 0 {
            reported += 1;
            println!("{:72} {:>4} {:>4}", entry.path, production_hits, test_hits);
            for sample in samples {
                println!("  {:72}  {}", "", sample);
            }
        }
    }
    if reported == 0 {
        println!("{:72} {:>4} {:>4}", "(no uses of any denied API)", 0, 0);
    }
    println!();
    println!("production blocking-API call sites: {production_total}");
    println!("test-context blocking-API call sites: {test_total}");
    println!("deny-list entries: {}", entries.len());
    Ok(())
}

fn walk_rs(root: PathBufLike) -> Result<Vec<std::path::PathBuf>, String> {
    let mut found = Vec::new();
    walk(&root, &mut found)?;
    Ok(found)
}

type PathBufLike = std::path::PathBuf;

fn walk(dir: &Path, found: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("blocking-census: read dir {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("blocking-census: dir entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            walk(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list_parses_the_committed_list() {
        let toml = include_str!("../../../clippy.toml");
        let entries = parse_deny_list(toml);
        assert!(entries.len() >= 54, "deny list shrank: {}", entries.len());
        assert!(entries.iter().any(|entry| entry.tail == "thread::sleep"));
        assert!(entries.iter().any(|entry| entry.path == "std::sync::Mutex::lock"));
    }

    #[test]
    fn cfg_test_lines_count_as_test_context() {
        let raw = "\
use std::thread;

pub fn run() {
    thread::sleep(std::time::Duration::from_secs(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn sleeps() {
        thread::sleep(std::time::Duration::from_secs(1));
    }
}

pub fn after() {}
";
        let (production, test) = split_contexts(raw, false);
        let production_has_sleep = production
            .iter()
            .any(|(_, text)| text.contains("thread::sleep"));
        let test_has_sleep = test.iter().any(|(_, text)| text.contains("thread::sleep"));
        assert!(production_has_sleep, "production call must stay production");
        assert!(test_has_sleep, "cfg(test) call must move to test context");
        assert!(production.iter().any(|(_, text)| text.contains("after()")));
    }
}