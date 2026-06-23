#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::repo_root;
use regex::Regex;

const ADR_0035_EXAMPLE_PATH: &str = "docs/adr/0035-efficiency-and-simplification-roadmap.md";
const ADR_0035_EXAMPLE_MARKER: &str =
    concat!("compat-", "ADR0042-added-20260815-wire-v6-handshake");
const REQUIRED_METADATA_FIELDS: [&str; 5] = ["from", "to", "owner", "removeWhen", "validation"];
const VALID_SURFACES: [&str; 9] = [
    "cli", "wire", "bundle", "option", "test", "schema", "daemon", "broker", "provider",
];

fn git_tracked_files() -> Vec<String> {
    let root = repo_root();
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("-c")
        .arg("core.quotePath=false")
        .args(["ls-files", "-z", "--"])
        .output()
        .expect("run `git ls-files -z`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut files = BTreeSet::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        if let Ok(rel) = String::from_utf8(raw.to_vec()) {
            files.insert(rel);
        }
    }
    files.into_iter().collect()
}

fn read_tracked_text_file(rel: &str) -> Option<String> {
    let content = std::fs::read_to_string(repo_root().join(rel)).ok()?;
    if content.contains('\0') {
        return None;
    }
    Some(content)
}

fn is_valid_yyyymmdd(date: &str) -> bool {
    if date.len() != 8 || !date.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let year = match date[0..4].parse::<u32>() {
        Ok(year) => year,
        Err(_) => return false,
    };
    let month = match date[4..6].parse::<u32>() {
        Ok(month) => month,
        Err(_) => return false,
    };
    let day = match date[6..8].parse::<u32>() {
        Ok(day) => day,
        Err(_) => return false,
    };

    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return false,
    };
    year > 0 && (1..=max_day).contains(&day)
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}

fn metadata_window(lines: &[&str], line_index: usize) -> String {
    let start = line_index.saturating_sub(8);
    let end = (line_index + 9).min(lines.len());
    lines[start..end].join("\n")
}

fn missing_metadata_fields(window: &str) -> Vec<&'static str> {
    REQUIRED_METADATA_FIELDS
        .into_iter()
        .filter(|field| {
            let field_re = Regex::new(&format!(
                r"(?m)(^|[^A-Za-z0-9_-]){}\s*[:=]\s*\S",
                regex::escape(field)
            ))
            .expect("valid metadata-field regex");
            !field_re.is_match(window)
        })
        .collect()
}

#[test]
fn compat_adr_markers_are_well_formed_and_metadata_backed() {
    let marker_token = Regex::new(concat!("compat-", r#"ADR[^\s`'"\]\[(){}|,;:.]*"#))
        .expect("valid marker-token regex");
    let strict_marker = Regex::new(
        r"^compat-ADR(?P<adr>\d{4})-added-(?P<date>\d{8})-(?P<surface>[a-z]+)-(?P<slug>[a-z0-9]+(?:-[a-z0-9]+)*)$",
    )
    .expect("valid strict-marker regex");

    let mut violations = Vec::new();

    for rel in git_tracked_files() {
        let Some(content) = read_tracked_text_file(&rel) else {
            continue;
        };
        let lines: Vec<&str> = content.lines().collect();
        for (line_index, line) in lines.iter().enumerate() {
            for token in marker_token.find_iter(line).map(|matched| matched.as_str()) {
                if token == "compat-ADR" || token.contains(['<', '>']) {
                    continue;
                }
                if rel == ADR_0035_EXAMPLE_PATH && token == ADR_0035_EXAMPLE_MARKER {
                    continue;
                }

                let Some(captures) = strict_marker.captures(token) else {
                    violations.push(format!(
                        "{}:{}: malformed compat-ADR marker `{}`",
                        rel,
                        line_index + 1,
                        token
                    ));
                    continue;
                };

                let date = captures.name("date").expect("date capture").as_str();
                if !is_valid_yyyymmdd(date) {
                    violations.push(format!(
                        "{}:{}: compat-ADR marker `{}` has invalid added date `{}`",
                        rel,
                        line_index + 1,
                        token,
                        date
                    ));
                }

                let surface = captures.name("surface").expect("surface capture").as_str();
                if !VALID_SURFACES.contains(&surface) {
                    violations.push(format!(
                        "{}:{}: compat-ADR marker `{}` uses unknown surface `{}`",
                        rel,
                        line_index + 1,
                        token,
                        surface
                    ));
                }

                let window = metadata_window(&lines, line_index);
                let missing = missing_metadata_fields(&window);
                if !missing.is_empty() {
                    violations.push(format!(
                        "{}:{}: compat-ADR marker `{}` is missing nearby metadata fields: {}",
                        rel,
                        line_index + 1,
                        token,
                        missing.join(", ")
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "compat-ADR policy violations:\n{}",
        violations.join("\n")
    );
}
