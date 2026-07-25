//! Fragment assembler for the `changelog.d/` directory.
//!
//! Concurrent branches each drop one fragment file into `changelog.d/`
//! instead of appending to the shared `## [Unreleased]` block in
//! `CHANGELOG.md`. Every branch then writes a file no other branch touches,
//! so the changelog stops being a guaranteed merge conflict whenever more
//! than one branch is in flight.
//!
//! The fold is deterministic (fragments are consumed in file-name order),
//! idempotent (with no fragments present the changelog is left byte-identical),
//! and fail-closed: a fragment that does not parse aborts the run with the
//! offending file and line instead of dropping the entry on the floor.

use std::{fmt, fs, path::Path};

/// Directory holding unfolded fragments, relative to the repository root.
pub const FRAGMENT_DIR: &str = "changelog.d";

/// Changelog the fragments fold into, relative to the repository root.
pub const CHANGELOG_FILE: &str = "CHANGELOG.md";

/// Heading of the block fragments fold into.
pub const UNRELEASED_HEADING: &str = "## [Unreleased]";

/// Fragment-directory file that documents the mechanism rather than
/// carrying entries, so the fold skips it.
const FRAGMENT_README: &str = "README.md";

/// Keep a Changelog section names, in canonical render order.
pub const SECTIONS: [&str; 6] = [
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];

/// Canonical order index of `title`, or `None` when it is not a Keep a
/// Changelog section name.
fn section_rank(title: &str) -> Option<usize> {
    SECTIONS.iter().position(|known| *known == title)
}

fn is_bullet(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ")
}

/// One or more reasons a fold cannot proceed. Every reason is reported, so a
/// batch of fragments does not have to be fixed one round-trip at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldError(Vec<String>);

impl FoldError {
    fn single(message: impl Into<String>) -> Self {
        Self(vec![message.into()])
    }

    /// The individual reasons, in report order.
    pub fn reasons(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (idx, reason) in self.0.iter().enumerate() {
            if idx > 0 {
                writeln!(f)?;
            }
            write!(f, "  - {reason}")?;
        }
        Ok(())
    }
}

impl std::error::Error for FoldError {}

/// One section of a parsed fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentSection {
    /// Canonical order index of the section name.
    pub rank: usize,
    /// Entry lines, verbatim, with surrounding blank lines removed.
    pub entries: Vec<String>,
}

/// A parsed `changelog.d/<name>.md` fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// Fragment file name, used for ordering and error messages.
    pub name: String,
    /// Sections, in canonical order, each non-empty.
    pub sections: Vec<FragmentSection>,
}

/// Parse one fragment, rejecting anything that could silently lose an entry.
///
/// Accepted shape: one or more `### <Section>` headings from [`SECTIONS`],
/// each followed by a bullet list. Rejected: an empty fragment, an unknown or
/// wrong-level heading, a repeated heading, a section with no entries, and any
/// content before the first heading.
pub fn parse_fragment(name: &str, text: &str) -> Result<Fragment, FoldError> {
    let mut errors: Vec<String> = Vec::new();
    let mut raw: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut current: Option<(usize, String, Vec<String>)> = None;
    let mut reported_stray_content = false;

    for (idx, source) in text.lines().enumerate() {
        let line = source.trim_end();
        let lineno = idx + 1;

        if line.starts_with('#') {
            match line.strip_prefix("### ") {
                Some(title) => {
                    if let Some(section) = current.take() {
                        raw.push(section);
                    }
                    current = Some((lineno, title.trim().to_string(), Vec::new()));
                }
                None => errors.push(format!(
                    "{name}:{lineno}: expected a '### <Section>' heading, found '{line}'"
                )),
            }
            continue;
        }

        match current.as_mut() {
            Some((_, _, lines)) => lines.push(line.to_string()),
            None => {
                if !line.trim().is_empty() && !reported_stray_content {
                    reported_stray_content = true;
                    errors.push(format!(
                        "{name}:{lineno}: content before the first '### <Section>' heading"
                    ));
                }
            }
        }
    }

    if let Some(section) = current.take() {
        raw.push(section);
    }

    if raw.is_empty() && errors.is_empty() {
        errors.push(format!(
            "{name}: no '### <Section>' heading; a fragment must carry at least one entry"
        ));
    }

    let mut sections: Vec<(usize, FragmentSection)> = Vec::new();
    let mut seen: Vec<(String, usize)> = Vec::new();

    for (lineno, title, lines) in raw {
        let Some(rank) = section_rank(&title) else {
            errors.push(format!(
                "{name}:{lineno}: unknown section '{title}'; expected one of {}",
                SECTIONS.join(", ")
            ));
            continue;
        };

        if let Some((_, first)) = seen.iter().find(|(known, _)| *known == title) {
            errors.push(format!(
                "{name}:{lineno}: duplicate '### {title}' heading (first seen on line {first})"
            ));
            continue;
        }
        seen.push((title.clone(), lineno));

        let entries = trim_blank_edges(lines);
        if entries.is_empty() {
            errors.push(format!(
                "{name}:{lineno}: section '### {title}' has no entries"
            ));
            continue;
        }
        if !is_bullet(&entries[0]) {
            errors.push(format!(
                "{name}:{lineno}: section '### {title}' must start with a '- ' bullet"
            ));
            continue;
        }

        sections.push((rank, FragmentSection { rank, entries }));
    }

    if !errors.is_empty() {
        return Err(FoldError(errors));
    }

    sections.sort_by_key(|(rank, _)| *rank);
    Ok(Fragment {
        name: name.to_string(),
        sections: sections.into_iter().map(|(_, section)| section).collect(),
    })
}

fn trim_blank_edges(mut lines: Vec<String>) -> Vec<String> {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// A `### <Section>` block of the existing `## [Unreleased]` region.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockSection {
    heading: String,
    title: String,
    lines: Vec<String>,
}

/// Fold `fragments` into the `## [Unreleased]` block of `changelog`.
///
/// Entries collate by section: every fragment's `### Added` bullets land under
/// the single `### Added` heading of the block, in fragment order, appended
/// after whatever the block already carried. Sections absent from the block are
/// inserted in canonical [`SECTIONS`] order. Released sections and every line
/// outside the block are copied through untouched.
pub fn fold_unreleased(changelog: &str, fragments: &[Fragment]) -> Result<String, FoldError> {
    let lines: Vec<&str> = changelog.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim_end() == UNRELEASED_HEADING)
        .ok_or_else(|| {
            FoldError::single(format!(
                "{CHANGELOG_FILE}: no '{UNRELEASED_HEADING}' heading to fold into"
            ))
        })?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| line.starts_with("## "))
        .map_or(lines.len(), |(idx, _)| idx);

    let mut preamble: Vec<String> = Vec::new();
    let mut sections: Vec<BlockSection> = Vec::new();
    for source in &lines[start + 1..end] {
        let line = source.trim_end();
        if let Some(title) = line.strip_prefix("### ") {
            sections.push(BlockSection {
                heading: line.to_string(),
                title: title.trim().to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        match sections.last_mut() {
            Some(section) => section.lines.push(line.to_string()),
            None => preamble.push(line.to_string()),
        }
    }
    preamble = trim_blank_edges(preamble);
    for section in &mut sections {
        section.lines = trim_blank_edges(std::mem::take(&mut section.lines));
    }

    for (rank, title) in SECTIONS.iter().enumerate() {
        let entries: Vec<String> = fragments
            .iter()
            .flat_map(|fragment| fragment.sections.iter())
            .filter(|section| section.rank == rank)
            .flat_map(|section| section.entries.iter().cloned())
            .collect();
        if entries.is_empty() {
            continue;
        }

        match sections.iter_mut().find(|section| section.title == *title) {
            Some(section) => section.lines.extend(entries),
            None => {
                let position = sections
                    .iter()
                    .position(|section| {
                        section_rank(&section.title).is_some_and(|other| other > rank)
                    })
                    .unwrap_or(sections.len());
                sections.insert(
                    position,
                    BlockSection {
                        heading: format!("### {title}"),
                        title: (*title).to_string(),
                        lines: entries,
                    },
                );
            }
        }
    }

    let mut out: Vec<String> = lines[..start]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    out.push(lines[start].to_string());
    if !preamble.is_empty() {
        out.push(String::new());
        out.extend(preamble);
    }
    for section in sections {
        out.push(String::new());
        out.push(section.heading);
        out.push(String::new());
        out.extend(section.lines);
    }
    if end < lines.len() {
        out.push(String::new());
        out.extend(lines[end..].iter().map(|line| (*line).to_string()));
    }

    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// Whether a fold writes its result or only validates the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Rewrite `CHANGELOG.md` and delete the consumed fragments.
    Apply,
    /// Parse the fragments and compute the fold without touching the tree.
    Check,
}

/// What a fold run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Fragment file names consumed, in fold order.
    pub folded: Vec<String>,
    /// Section name and entry-line count per merged section, in render order.
    pub sections: Vec<(String, usize)>,
}

/// Load every fragment in `<repo_root>/changelog.d`, in file-name order.
fn load_fragments(repo_root: &Path) -> Result<Vec<Fragment>, FoldError> {
    let dir = repo_root.join(FRAGMENT_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&dir).map_err(|err| {
        FoldError::single(format!("{FRAGMENT_DIR}: cannot read directory: {err}"))
    })?;

    let mut errors: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            FoldError::single(format!(
                "{FRAGMENT_DIR}: cannot read directory entry: {err}"
            ))
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == FRAGMENT_README {
            continue;
        }
        let is_file = entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false);
        if !is_file {
            errors.push(format!(
                "{FRAGMENT_DIR}/{name}: not a regular file; the fragment directory holds one '.md' file per branch"
            ));
            continue;
        }
        if !name.ends_with(".md") {
            errors.push(format!(
                "{FRAGMENT_DIR}/{name}: fragments must be named '<branch>.md'"
            ));
            continue;
        }
        names.push(name);
    }

    if !errors.is_empty() {
        return Err(FoldError(errors));
    }
    names.sort();

    let mut fragments: Vec<Fragment> = Vec::new();
    for name in names {
        let path = dir.join(&name);
        match fs::read_to_string(&path) {
            Ok(text) => match parse_fragment(&name, &text) {
                Ok(fragment) => fragments.push(fragment),
                Err(err) => errors.extend(err.0),
            },
            Err(err) => errors.push(format!("{FRAGMENT_DIR}/{name}: cannot read: {err}")),
        }
    }

    if errors.is_empty() {
        Ok(fragments)
    } else {
        Err(FoldError(errors))
    }
}

/// Fold every fragment under `repo_root` into the repository changelog.
///
/// With no fragments present nothing is read, written, or deleted, so a second
/// run is a no-op.
pub fn fold_repo(repo_root: &Path, mode: Mode) -> Result<Outcome, FoldError> {
    let fragments = load_fragments(repo_root)?;
    if fragments.is_empty() {
        return Ok(Outcome {
            folded: Vec::new(),
            sections: Vec::new(),
        });
    }

    let changelog_path = repo_root.join(CHANGELOG_FILE);
    let changelog = fs::read_to_string(&changelog_path)
        .map_err(|err| FoldError::single(format!("{CHANGELOG_FILE}: cannot read: {err}")))?;
    let folded = fold_unreleased(&changelog, &fragments)?;

    let mut sections: Vec<(String, usize)> = Vec::new();
    for (rank, title) in SECTIONS.iter().enumerate() {
        let count: usize = fragments
            .iter()
            .flat_map(|fragment| fragment.sections.iter())
            .filter(|section| section.rank == rank)
            .map(|section| section.entries.len())
            .sum();
        if count > 0 {
            sections.push(((*title).to_string(), count));
        }
    }

    let outcome = Outcome {
        folded: fragments
            .iter()
            .map(|fragment| fragment.name.clone())
            .collect(),
        sections,
    };

    if mode == Mode::Check {
        return Ok(outcome);
    }

    apply_fold(repo_root, &changelog_path, &folded, &outcome.folded)?;

    Ok(outcome)
}

/// Commit a computed fold to disk atomically, with rollback on any failure.
///
/// The changelog rewrite and the fragment removals are made all-or-nothing so a
/// partial write or a failed removal can never leave a corrupted changelog,
/// duplicated entries on retry, or a half-consumed fragment set. The steps:
///
/// 1. Stage the new changelog into a private staging directory on the same
///    filesystem as the target (a rename across it is atomic).
/// 2. Reserve every consumed fragment by moving it into the staging directory
///    *before* the changelog is promoted, so a failure here rolls back to the
///    original tree byte-for-byte.
/// 3. Promote the staged changelog over `CHANGELOG.md` with a single atomic
///    rename.
/// 4. Discard the staging directory. It lives outside `changelog.d/`, so even a
///    failed final cleanup cannot poison a later fold (the fragments are already
///    gone from their canonical names, so a retry is a no-op).
fn apply_fold(
    repo_root: &Path,
    changelog_path: &Path,
    folded: &str,
    folded_names: &[String],
) -> Result<(), FoldError> {
    let fragment_dir = repo_root.join(FRAGMENT_DIR);
    let staging = repo_root.join(format!(".changelog-fold.{}.tmp", std::process::id()));

    // A stale staging directory from an interrupted earlier run must not leak
    // into this one.
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging).map_err(|err| {
        FoldError::single(format!(
            "{CHANGELOG_FILE}: cannot create staging directory: {err}"
        ))
    })?;

    let staged_changelog = staging.join("CHANGELOG.md.new");
    if let Err(err) = fs::write(&staged_changelog, folded) {
        let _ = fs::remove_dir_all(&staging);
        return Err(FoldError::single(format!(
            "{CHANGELOG_FILE}: cannot stage rewrite: {err}"
        )));
    }

    // Reserve fragments by moving them aside. `reserved` records the moves so
    // they can be undone in reverse if a later step fails.
    let mut reserved: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    let rollback = |reserved: &[(std::path::PathBuf, std::path::PathBuf)]| {
        for (canonical, aside) in reserved.iter().rev() {
            let _ = fs::rename(aside, canonical);
        }
        let _ = fs::remove_dir_all(&staging);
    };

    for name in folded_names {
        let canonical = fragment_dir.join(name);
        let aside = staging.join(name);
        if let Err(err) = fs::rename(&canonical, &aside) {
            rollback(&reserved);
            return Err(FoldError::single(format!(
                "{FRAGMENT_DIR}/{name}: cannot reserve for removal: {err}"
            )));
        }
        reserved.push((canonical, aside));
    }

    // Promote the rewrite. Only now does CHANGELOG.md change.
    if let Err(err) = fs::rename(&staged_changelog, changelog_path) {
        rollback(&reserved);
        return Err(FoldError::single(format!(
            "{CHANGELOG_FILE}: cannot promote rewrite: {err}"
        )));
    }

    // The fold is committed. The reserved fragments are already gone from their
    // canonical names, so discarding the staging directory is pure cleanup; a
    // failure here cannot corrupt the changelog or double-fold on retry.
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

/// `cargo xtask changelog-fold [--check]`.
pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    let mode = match args {
        [] => Mode::Apply,
        [flag] if flag == "--check" => Mode::Check,
        _ => {
            eprintln!("usage: cargo xtask changelog-fold [--check]");
            return std::process::ExitCode::FAILURE;
        }
    };

    let repo_root = match crate::repo_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("changelog-fold failed: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match fold_repo(repo_root, mode) {
        Ok(outcome) if outcome.folded.is_empty() => {
            println!("changelog-fold: no fragments in {FRAGMENT_DIR}/; {CHANGELOG_FILE} unchanged");
            std::process::ExitCode::SUCCESS
        }
        Ok(outcome) => {
            let verb = match mode {
                Mode::Apply => "folded",
                Mode::Check => "would fold",
            };
            println!(
                "changelog-fold: {verb} {} fragment(s) into {CHANGELOG_FILE} {UNRELEASED_HEADING}",
                outcome.folded.len()
            );
            for name in &outcome.folded {
                println!("  {FRAGMENT_DIR}/{name}");
            }
            for (title, count) in &outcome.sections {
                println!("  ### {title}: {count} line(s)");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("changelog-fold failed:");
            for reason in err.reasons() {
                eprintln!("  - {reason}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fragment(name: &str, text: &str) -> Fragment {
        parse_fragment(name, text).expect("fragment parses")
    }

    #[test]
    fn parses_sections_in_canonical_order() {
        let parsed = fragment(
            "b.md",
            "### Fixed\n\n- fixed one\n\n### Added\n\n- added one\n- added two\n",
        );
        assert_eq!(parsed.name, "b.md");
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[0].rank, section_rank("Added").unwrap());
        assert_eq!(
            parsed.sections[0].entries,
            vec!["- added one", "- added two"]
        );
        assert_eq!(parsed.sections[1].rank, section_rank("Fixed").unwrap());
        assert_eq!(parsed.sections[1].entries, vec!["- fixed one"]);
    }

    #[test]
    fn preserves_multi_line_entries_verbatim() {
        let parsed = fragment(
            "a.md",
            "### Added\n\n- first line\n  continued line\n\n- second entry\n",
        );
        assert_eq!(
            parsed.sections[0].entries,
            vec!["- first line", "  continued line", "", "- second entry"]
        );
    }

    #[test]
    fn rejects_empty_fragment() {
        let err = parse_fragment("a.md", "").expect_err("empty fragment is rejected");
        assert!(
            err.reasons()[0].contains("no '### <Section>' heading"),
            "{err}"
        );
    }

    #[test]
    fn rejects_unknown_section() {
        let err =
            parse_fragment("a.md", "### Improved\n\n- entry\n").expect_err("unknown section fails");
        assert!(
            err.reasons()[0].contains("unknown section 'Improved'"),
            "{err}"
        );
    }

    #[test]
    fn rejects_wrong_heading_level() {
        let err = parse_fragment("a.md", "## Added\n\n- entry\n").expect_err("wrong level fails");
        assert!(
            err.reasons()[0].contains("expected a '### <Section>' heading"),
            "{err}"
        );
    }

    #[test]
    fn rejects_content_outside_a_section() {
        let err = parse_fragment("a.md", "stray prose\n\n### Added\n\n- entry\n")
            .expect_err("stray content fails");
        assert!(
            err.reasons()[0].contains("content before the first '### <Section>' heading"),
            "{err}"
        );
    }

    #[test]
    fn rejects_empty_section() {
        let err = parse_fragment("a.md", "### Added\n\n### Fixed\n\n- entry\n")
            .expect_err("empty section fails");
        assert!(err.reasons()[0].contains("has no entries"), "{err}");
    }

    #[test]
    fn rejects_duplicate_section() {
        let err = parse_fragment("a.md", "### Added\n\n- one\n\n### Added\n\n- two\n")
            .expect_err("duplicate section fails");
        assert!(
            err.reasons()[0].contains("duplicate '### Added' heading"),
            "{err}"
        );
    }

    #[test]
    fn rejects_non_bullet_section_body() {
        let err = parse_fragment("a.md", "### Added\n\nprose, not a bullet\n")
            .expect_err("non-bullet body fails");
        assert!(
            err.reasons()[0].contains("must start with a '- ' bullet"),
            "{err}"
        );
    }

    #[test]
    fn reports_every_reason_in_one_pass() {
        let err = parse_fragment("a.md", "### Improved\n\n- one\n\n### Added\n")
            .expect_err("both problems fail");
        assert_eq!(err.reasons().len(), 2, "{err}");
    }

    const CHANGELOG: &str = concat!(
        "# Changelog\n",
        "\n",
        "## [Unreleased]\n",
        "\n",
        "### Added\n",
        "\n",
        "- existing added entry\n",
        "\n",
        "### Security\n",
        "\n",
        "- existing security entry\n",
        "\n",
        "## [1.0.0] - 2026-01-01\n",
        "\n",
        "### Added\n",
        "\n",
        "- released entry\n",
    );

    #[test]
    fn collates_into_existing_sections_and_inserts_new_ones_in_canonical_order() {
        let fragments = vec![
            fragment("a.md", "### Added\n\n- from a\n"),
            fragment(
                "b.md",
                "### Added\n\n- from b\n\n### Fixed\n\n- fix from b\n",
            ),
        ];
        let folded = fold_unreleased(CHANGELOG, &fragments).expect("fold succeeds");
        assert_eq!(
            folded,
            concat!(
                "# Changelog\n",
                "\n",
                "## [Unreleased]\n",
                "\n",
                "### Added\n",
                "\n",
                "- existing added entry\n",
                "- from a\n",
                "- from b\n",
                "\n",
                "### Fixed\n",
                "\n",
                "- fix from b\n",
                "\n",
                "### Security\n",
                "\n",
                "- existing security entry\n",
                "\n",
                "## [1.0.0] - 2026-01-01\n",
                "\n",
                "### Added\n",
                "\n",
                "- released entry\n",
            )
        );
    }

    #[test]
    fn leaves_released_sections_untouched() {
        let fragments = vec![fragment("a.md", "### Removed\n\n- removed something\n")];
        let folded = fold_unreleased(CHANGELOG, &fragments).expect("fold succeeds");
        let released = folded
            .split_once("## [1.0.0] - 2026-01-01\n")
            .expect("released section survives")
            .1;
        assert_eq!(released, "\n### Added\n\n- released entry\n");
    }

    #[test]
    fn preserves_block_preamble_and_unknown_sections() {
        let changelog = concat!(
            "## [Unreleased]\n",
            "\n",
            "Prose that introduces the block.\n",
            "\n",
            "### Notes\n",
            "\n",
            "- an entry under a non-canonical heading\n",
        );
        let fragments = vec![fragment("a.md", "### Added\n\n- new entry\n")];
        let folded = fold_unreleased(changelog, &fragments).expect("fold succeeds");
        assert_eq!(
            folded,
            concat!(
                "## [Unreleased]\n",
                "\n",
                "Prose that introduces the block.\n",
                "\n",
                "### Notes\n",
                "\n",
                "- an entry under a non-canonical heading\n",
                "\n",
                "### Added\n",
                "\n",
                "- new entry\n",
            )
        );
    }

    #[test]
    fn folds_into_an_empty_unreleased_block() {
        let changelog = concat!(
            "## [Unreleased]\n",
            "\n",
            "## [1.0.0] - 2026-01-01\n",
            "\n",
            "- released entry\n",
        );
        let fragments = vec![fragment("a.md", "### Changed\n\n- changed something\n")];
        let folded = fold_unreleased(changelog, &fragments).expect("fold succeeds");
        assert_eq!(
            folded,
            concat!(
                "## [Unreleased]\n",
                "\n",
                "### Changed\n",
                "\n",
                "- changed something\n",
                "\n",
                "## [1.0.0] - 2026-01-01\n",
                "\n",
                "- released entry\n",
            )
        );
    }

    #[test]
    fn fold_order_follows_fragment_order_only() {
        let a = fragment("a.md", "### Added\n\n- from a\n");
        let b = fragment("b.md", "### Added\n\n- from b\n");
        let forward = fold_unreleased(CHANGELOG, &[a.clone(), b.clone()]).expect("fold succeeds");
        let reversed = fold_unreleased(CHANGELOG, &[b, a]).expect("fold succeeds");
        assert!(forward.contains("- from a\n- from b\n"), "{forward}");
        assert!(reversed.contains("- from b\n- from a\n"), "{reversed}");
    }

    #[test]
    fn refuses_a_changelog_without_an_unreleased_block() {
        let err = fold_unreleased(
            "# Changelog\n\n## [1.0.0] - 2026-01-01\n",
            &[fragment("a.md", "### Added\n\n- entry\n")],
        )
        .expect_err("missing block fails");
        assert!(
            err.reasons()[0].contains("no '## [Unreleased]' heading"),
            "{err}"
        );
    }

    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A throwaway repository tree under the gitignored `.agent-tmp/` scratch
    /// root, cleaned up on drop even when a test panics.
    struct TempRepo {
        root: PathBuf,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("repo root above packages/xtask")
                .to_path_buf();
            let base = repo_root.join(".agent-tmp").join("xtask-changelog");
            fs::create_dir_all(&base).expect("create scratch base");
            let unique = format!(
                "{tag}.{}.{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let root = base.join(unique);
            fs::create_dir_all(root.join(FRAGMENT_DIR)).expect("create changelog.d");
            TempRepo { root }
        }

        fn write_changelog(&self, body: &str) {
            fs::write(self.root.join(CHANGELOG_FILE), body).expect("write changelog");
        }

        fn write_fragment(&self, name: &str, body: &str) {
            fs::write(self.root.join(FRAGMENT_DIR).join(name), body).expect("write fragment");
        }

        fn changelog(&self) -> String {
            fs::read_to_string(self.root.join(CHANGELOG_FILE)).expect("read changelog")
        }

        fn fragment_names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.root.join(FRAGMENT_DIR))
                .expect("read changelog.d")
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o755));
            let _ = fs::set_permissions(
                self.root.join(FRAGMENT_DIR),
                fs::Permissions::from_mode(0o755),
            );
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn load_fragments_rejects_a_non_md_regular_file() {
        let repo = TempRepo::new("non-md");
        repo.write_fragment("valid.md", "### Added\n\n- ok\n");
        repo.write_fragment("notes.txt", "loose text file\n");
        let err = load_fragments(&repo.root).expect_err("non-.md entry rejected");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("notes.txt") && reason.contains("must be named")),
            "{err}"
        );
    }

    #[test]
    fn load_fragments_rejects_a_symlink_fragment() {
        let repo = TempRepo::new("symlink");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("real.md", "### Added\n\n- ok\n");
        std::os::unix::fs::symlink(
            repo.root.join(CHANGELOG_FILE),
            repo.root.join(FRAGMENT_DIR).join("link.md"),
        )
        .expect("create symlink");
        let err = load_fragments(&repo.root).expect_err("symlink rejected");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("link.md") && reason.contains("not a regular file")),
            "{err}"
        );
    }

    #[test]
    fn load_fragments_rejects_invalid_utf8() {
        let repo = TempRepo::new("utf8");
        fs::write(
            repo.root.join(FRAGMENT_DIR).join("bad.md"),
            [0xffu8, 0xfe, 0x00, 0x41],
        )
        .expect("write non-utf8 bytes");
        let err = load_fragments(&repo.root).expect_err("invalid utf-8 rejected");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("bad.md") && reason.contains("cannot read")),
            "{err}"
        );
    }

    #[test]
    fn load_fragments_skips_readme_and_accepts_valid_fragments() {
        let repo = TempRepo::new("readme");
        repo.write_fragment(FRAGMENT_README, "not a fragment, just docs\n");
        repo.write_fragment("change.md", "### Added\n\n- ok\n");
        let fragments = load_fragments(&repo.root).expect("valid load");
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].name, "change.md");
    }

    #[test]
    fn fold_repo_applies_rewrite_and_consumes_every_fragment() {
        let repo = TempRepo::new("apply");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
        repo.write_fragment("feature-b.md", "### Fixed\n\n- fix from b\n");

        let outcome = fold_repo(&repo.root, Mode::Apply).expect("fold applies");
        assert_eq!(outcome.folded.len(), 2);

        let folded = repo.changelog();
        assert!(folded.contains("- from a\n"), "{folded}");
        assert!(folded.contains("- fix from b\n"), "{folded}");
        assert!(repo.fragment_names().is_empty(), "fragments consumed");

        let again = fold_repo(&repo.root, Mode::Apply).expect("second run is a no-op");
        assert!(again.folded.is_empty());
        assert_eq!(
            repo.changelog(),
            folded,
            "second run leaves changelog stable"
        );
    }

    #[test]
    fn fold_repo_rolls_back_when_a_fragment_cannot_be_reserved() {
        let repo = TempRepo::new("reserve-fail");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
        let before = repo.changelog();

        let dir = repo.root.join(FRAGMENT_DIR);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        // Root bypasses directory permission bits; the injection is a no-op then.
        if fs::rename(dir.join("feature-a.md"), dir.join(".probe")).is_ok() {
            let _ = fs::rename(dir.join(".probe"), dir.join("feature-a.md"));
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipping fold_repo reservation-failure test: running as root");
            return;
        }

        let err = fold_repo(&repo.root, Mode::Apply).expect_err("reservation fails");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("cannot reserve")),
            "{err}"
        );

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(repo.changelog(), before, "changelog left byte-unchanged");
        assert_eq!(
            repo.fragment_names(),
            vec!["feature-a.md".to_string()],
            "fragment left intact"
        );
    }

    #[test]
    fn fold_repo_leaves_tree_unchanged_when_staging_cannot_be_created() {
        let repo = TempRepo::new("stage-fail");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
        let before = repo.changelog();

        fs::set_permissions(&repo.root, fs::Permissions::from_mode(0o555)).unwrap();
        // Root bypasses directory permission bits; the injection is a no-op then.
        if fs::create_dir(repo.root.join(".probe-writable")).is_ok() {
            let _ = fs::remove_dir(repo.root.join(".probe-writable"));
            fs::set_permissions(&repo.root, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipping fold_repo staging-failure test: running as root");
            return;
        }

        let err = fold_repo(&repo.root, Mode::Apply).expect_err("staging fails");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("staging directory")),
            "{err}"
        );

        fs::set_permissions(&repo.root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(repo.changelog(), before, "changelog left byte-unchanged");
        assert_eq!(
            repo.fragment_names(),
            vec!["feature-a.md".to_string()],
            "fragment left intact"
        );
    }
}
