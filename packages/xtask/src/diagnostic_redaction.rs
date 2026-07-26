//! Path-safe filtering for diagnostics that must reach operator stderr.

use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const DEFAULT_TAIL_LINES: usize = 20;
const MAX_DIAGNOSTIC_BYTES: u64 = 4 * 1024 * 1024;

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
        roots.sort_by(|left, right| right.path.len().cmp(&left.path.len()));
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
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(MAX_DIAGNOSTIC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RedactionError("cannot read the diagnostic input"))?;
    if bytes.len() as u64 > MAX_DIAGNOSTIC_BYTES {
        return Err(RedactionError(
            "diagnostic input exceeds the safe size limit",
        ));
    }
    let diagnostic =
        std::str::from_utf8(&bytes).map_err(|_| RedactionError("diagnostic input is not UTF-8"))?;
    let redacted = tail_lines(&redactor.redact(diagnostic), tail);
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
}
