//! Path-safe filtering for diagnostics that must reach operator stderr.

use std::{
    collections::VecDeque,
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const DEFAULT_TAIL_LINES: usize = 20;
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RedactionError(&'static str);

impl std::fmt::Display for RedactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

struct SensitiveRoot {
    path: String,
    replacement: &'static str,
}

struct DiagnosticRedactor {
    roots: Vec<SensitiveRoot>,
}

impl DiagnosticRedactor {
    fn new(repo_root: &Path, home: Option<&Path>) -> Result<Self, RedactionError> {
        let mut roots = Vec::new();
        add_sensitive_root(&mut roots, repo_root, "<repo>", "repository root")?;
        if let Some(home) = home {
            add_sensitive_root(&mut roots, home, "<home>", "home directory")?;
        }
        roots.sort_by_key(|root| std::cmp::Reverse(root.path.len()));
        roots.dedup_by(|left, right| left.path == right.path);
        Ok(Self { roots })
    }

    fn redact(&self, input: &str) -> String {
        let mut redacted = input.to_owned();
        for root in &self.roots {
            redacted = replace_sensitive_root(&redacted, &root.path, root.replacement);
        }
        redact_other_absolute_paths(&strip_control_characters(&redacted))
    }
}

pub(crate) fn redact_path(path: &Path) -> String {
    let Ok(repo_root) = crate::repo_root() else {
        return "<path>".to_owned();
    };
    let Ok(redactor) = DiagnosticRedactor::new(repo_root, None) else {
        return "<path>".to_owned();
    };
    let Some(path) = path.to_str() else {
        return "<path>".to_owned();
    };
    redactor.redact(path)
}

fn add_sensitive_root(
    roots: &mut Vec<SensitiveRoot>,
    supplied: &Path,
    replacement: &'static str,
    role: &'static str,
) -> Result<(), RedactionError> {
    let canonical = fs::canonicalize(supplied).map_err(|_| match role {
        "repository root" => RedactionError("cannot resolve the repository root"),
        _ => RedactionError("cannot resolve the home directory"),
    })?;
    let canonical = canonical
        .to_str()
        .ok_or(RedactionError("a sensitive path is not valid UTF-8"))?;
    roots.push(SensitiveRoot {
        path: normalized_root(canonical),
        replacement,
    });

    let lexical = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|_| RedactionError("cannot resolve the current directory"))?
            .join(supplied)
    };
    let lexical = lexical
        .to_str()
        .ok_or(RedactionError("a sensitive path is not valid UTF-8"))?;
    let lexical = normalized_root(lexical);
    if lexical != canonical {
        roots.push(SensitiveRoot {
            path: lexical,
            replacement,
        });
    }
    Ok(())
}

fn normalized_root(path: &str) -> String {
    if path == "/" {
        path.to_owned()
    } else {
        path.trim_end_matches('/').to_owned()
    }
}

fn is_start_boundary(bytes: &[u8], index: usize) -> bool {
    index == 0
        || matches!(
            bytes[index - 1],
            b' ' | b'\t' | b'\n' | b'\r' | b'\'' | b'"' | b'(' | b'[' | b'{' | b'=' | b':'
        )
}

fn is_end_boundary(bytes: &[u8], index: usize) -> bool {
    index == bytes.len()
        || matches!(
            bytes[index],
            b'/' | b' '
                | b'\t'
                | b'\n'
                | b'\r'
                | b'\''
                | b'"'
                | b')'
                | b']'
                | b'}'
                | b':'
                | b','
                | b';'
        )
}

fn replace_sensitive_root(input: &str, root: &str, replacement: &str) -> String {
    if root.is_empty() {
        return input.to_owned();
    }
    let mut output = String::with_capacity(input.len());
    let mut copied_through = 0;
    let mut search_from = 0;
    while let Some(offset) = input[search_from..].find(root) {
        let start = search_from + offset;
        let end = start + root.len();
        if is_start_boundary(input.as_bytes(), start) && is_end_boundary(input.as_bytes(), end) {
            output.push_str(&input[copied_through..start]);
            output.push_str(replacement);
            copied_through = end;
            search_from = end;
        } else {
            search_from = start + 1;
        }
    }
    output.push_str(&input[copied_through..]);
    output
}

fn strip_control_characters(input: &str) -> String {
    input
        .chars()
        .map(|character| match character {
            '\n' | '\t' => character,
            _ if character.is_control() => ' ',
            _ => character,
        })
        .collect()
}

fn is_unquoted_path_terminator(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'\'' | b'"' | b')' | b']' | b'}' | b'<' | b'>' | b',' | b';' | b'|'
        )
}

fn redact_other_absolute_paths(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut copied_through = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && is_start_boundary(bytes, index) {
            let mut end = index + 1;
            while end < bytes.len() && !is_unquoted_path_terminator(bytes[end]) {
                end += input[end..].chars().next().map_or(1, char::len_utf8);
            }
            output.push_str(&input[copied_through..index]);
            output.push_str("<path>");
            copied_through = end;
            index = end;
        } else {
            index += input[index..].chars().next().map_or(1, char::len_utf8);
        }
    }
    output.push_str(&input[copied_through..]);
    output
}

fn tail_lines(input: &str, count: usize) -> String {
    if count == 0 || input.is_empty() {
        return String::new();
    }
    let lines: Vec<_> = input.lines().collect();
    let start = lines.len().saturating_sub(count);
    let mut tail = lines[start..].join("\n");
    if !tail.is_empty() {
        tail.push('\n');
    }
    tail
}

fn read_diagnostic_tail(mut input: impl Read) -> Result<(Vec<u8>, u64), RedactionError> {
    let mut tail = VecDeque::with_capacity(MAX_DIAGNOSTIC_BYTES);
    let mut buffer = [0u8; 8192];
    let mut total_bytes = 0u64;
    let mut last_dropped = None;

    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| RedactionError("cannot read the diagnostic input"))?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(read as u64);
        let overflow = tail
            .len()
            .saturating_add(read)
            .saturating_sub(MAX_DIAGNOSTIC_BYTES);
        for _ in 0..overflow {
            last_dropped = tail.pop_front();
        }
        tail.extend(&buffer[..read]);
    }

    let mut tail: Vec<u8> = tail.into();
    if let Some(last_dropped) = last_dropped
        && !is_start_boundary(&[last_dropped], 1)
    {
        let boundary = tail
            .iter()
            .enumerate()
            .find_map(|(index, _)| is_start_boundary(&tail, index + 1).then_some(index + 1));
        match boundary {
            Some(boundary) => {
                tail.drain(..boundary);
            }
            None => tail.clear(),
        };
    }
    let dropped_bytes = total_bytes.saturating_sub(tail.len() as u64);
    Ok((tail, dropped_bytes))
}

fn parse_args(args: &[String]) -> Result<(PathBuf, Option<PathBuf>, usize), RedactionError> {
    let mut repo_root = None;
    let mut home = None;
    let mut tail = DEFAULT_TAIL_LINES;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo-root" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or(RedactionError("--repo-root requires a value"))?;
                if repo_root.replace(PathBuf::from(value)).is_some() {
                    return Err(RedactionError("--repo-root may be specified only once"));
                }
            }
            "--home" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or(RedactionError("--home requires a value"))?;
                if home.replace(PathBuf::from(value)).is_some() {
                    return Err(RedactionError("--home may be specified only once"));
                }
            }
            "--tail-lines" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or(RedactionError("--tail-lines requires a value"))?;
                tail = value
                    .parse()
                    .map_err(|_| RedactionError("--tail-lines requires a non-negative integer"))?;
            }
            _ => return Err(RedactionError("unknown diagnostic-redaction option")),
        }
        index += 1;
    }
    let repo_root = repo_root.ok_or(RedactionError("--repo-root is required"))?;
    Ok((repo_root, home, tail))
}

fn filter(
    args: &[String],
    mut input: impl Read,
    mut output: impl Write,
) -> Result<(), RedactionError> {
    let (repo_root, home, tail) = parse_args(args)?;
    let redactor = DiagnosticRedactor::new(&repo_root, home.as_deref())?;
    let (bytes, dropped_bytes) = read_diagnostic_tail(input.by_ref())?;
    let diagnostic = String::from_utf8_lossy(&bytes);
    let redacted = tail_lines(&redactor.redact(&diagnostic), tail);
    if dropped_bytes > 0 {
        writeln!(
            output,
            "diagnostic truncated: dropped {dropped_bytes} byte(s) from the beginning; \
             showing the redacted tail"
        )
        .map_err(|_| RedactionError("cannot write the redacted diagnostic"))?;
    }
    output
        .write_all(redacted.as_bytes())
        .map_err(|_| RedactionError("cannot write the redacted diagnostic"))
}

pub(crate) fn run_cli(args: &[String]) -> ExitCode {
    match filter(args, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("diagnostic redaction failed ({error}); raw diagnostic output suppressed");
            ExitCode::from(74)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicU32, Ordering},
    };

    static SCRATCH_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let target = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .parent()
                        .expect("xtask has a workspace parent")
                        .join("target")
                });
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = target
                .join("diagnostic-redaction-tests")
                .join(format!("{label}-{}-{sequence}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch directory is creatable");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_longer_sibling_is_not_partially_rewritten() {
        let scratch = Scratch::new("sibling-prefix");
        let home = scratch.0.join("paydro");
        let repo = home.join("project");
        fs::create_dir_all(&repo).unwrap();
        let redactor = DiagnosticRedactor::new(&repo, Some(&home)).unwrap();
        let sibling = format!("{}dro/x", home.display());
        let diagnostic = format!("{sibling} and {}/y", home.display());
        assert_eq!(redactor.redact(&diagnostic), "<path> and <home>/y");
    }

    #[test]
    fn metacharacters_are_matched_literally() {
        let scratch = Scratch::new("literal-metacharacters");
        let home = scratch.0.join("home#[literal]");
        let repo = home.join("repo#checkout[1]");
        fs::create_dir_all(&repo).unwrap();
        let redactor = DiagnosticRedactor::new(&repo, Some(&home)).unwrap();
        let diagnostic = format!("{}/src/main.rs {}", repo.display(), home.display());
        assert_eq!(redactor.redact(&diagnostic), "<repo>/src/main.rs <home>");
    }

    #[test]
    fn canonical_paths_from_a_symlinked_checkout_are_redacted() {
        let scratch = Scratch::new("symlink-checkout");
        let real = scratch.0.join("real/repo");
        let link = scratch.0.join("checkout-link");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, &link).unwrap();
        let redactor = DiagnosticRedactor::new(&link, None).unwrap();
        let diagnostic = format!("{}/src/lib.rs", real.display());
        assert_eq!(redactor.redact(&diagnostic), "<repo>/src/lib.rs");
    }

    #[test]
    fn central_path_redaction_preserves_repository_context() {
        let repo = crate::repo_root().unwrap();
        assert_eq!(
            redact_path(&repo.join("one/shared.rs")),
            "<repo>/one/shared.rs"
        );
        assert_eq!(
            redact_path(&repo.join("two/shared.rs")),
            "<repo>/two/shared.rs"
        );
    }

    #[test]
    fn filtering_fails_before_emitting_raw_input_when_a_root_cannot_resolve() {
        let scratch = Scratch::new("fail-closed");
        let missing = scratch.0.join("missing");
        let args = vec![
            "--repo-root".to_owned(),
            missing.display().to_string(),
            "--tail-lines".to_owned(),
            "20".to_owned(),
        ];
        let mut output = Vec::new();
        assert!(filter(&args, &b"/home/alice/private"[..], &mut output).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn oversized_input_emits_a_redacted_tail_and_truncation_notice() {
        let scratch = Scratch::new("oversized");
        let repo = scratch.0.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let args = vec![
            "--repo-root".to_owned(),
            repo.display().to_string(),
            "--tail-lines".to_owned(),
            "2".to_owned(),
        ];
        let mut input = vec![b'x'; MAX_DIAGNOSTIC_BYTES + 128];
        input.push(b'\n');
        input.extend(format!("error: {}/src/tail.rs\n", repo.display()).bytes());
        let expected_dropped = MAX_DIAGNOSTIC_BYTES + 129;
        let mut output = Vec::new();

        filter(&args, input.as_slice(), &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(&format!(
                "diagnostic truncated: dropped {expected_dropped} byte(s)"
            )),
            "the truncation notice must quantify the discarded prefix: {output}"
        );
        assert!(
            output.contains("error: <repo>/src/tail.rs"),
            "the retained tail must remain useful and redacted: {output}"
        );
        assert!(!output.contains(repo.to_str().unwrap()));
    }

    #[test]
    fn truncation_inside_a_multibyte_character_still_emits_a_redacted_tail() {
        let scratch = Scratch::new("multibyte-boundary");
        let repo = scratch.0.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let args = vec![
            "--repo-root".to_owned(),
            repo.display().to_string(),
            "--tail-lines".to_owned(),
            "1".to_owned(),
        ];
        let operative = format!("\nerror: {}/src/tail.rs\n", repo.display());
        let filler_len = MAX_DIAGNOSTIC_BYTES - 1 - operative.len();
        let mut input = vec![0xc3, 0xa9];
        input.extend(std::iter::repeat_n(b'x', filler_len));
        input.extend(operative.bytes());
        assert_eq!(input.len(), MAX_DIAGNOSTIC_BYTES + 1);
        let mut output = Vec::new();

        filter(&args, input.as_slice(), &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("diagnostic truncated: dropped"),
            "the split UTF-8 prefix must produce a truncation notice: {output}"
        );
        assert!(
            output.contains("error: <repo>/src/tail.rs"),
            "the retained tail must survive the split and remain redacted: {output}"
        );
        assert!(!output.contains(repo.to_str().unwrap()));
    }

    #[test]
    fn malformed_bytes_in_the_retained_tail_do_not_suppress_redacted_output() {
        let scratch = Scratch::new("malformed-retained-tail");
        let repo = scratch.0.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let args = vec![
            "--repo-root".to_owned(),
            repo.display().to_string(),
            "--tail-lines".to_owned(),
            "1".to_owned(),
        ];
        let mut input = vec![0xff, b'\n'];
        input.extend(format!("error: {}/src/tail.rs\n", repo.display()).bytes());
        let mut output = Vec::new();

        filter(&args, input.as_slice(), &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("error: <repo>/src/tail.rs"), "{output}");
        assert!(!output.contains(repo.to_str().unwrap()));
    }
}
