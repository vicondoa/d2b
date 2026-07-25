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

use std::{fmt, fs, io::Write, path::Path};

/// Directory holding unfolded fragments, relative to the repository root.
pub const FRAGMENT_DIR: &str = "changelog.d";

/// Changelog the fragments fold into, relative to the repository root.
pub const CHANGELOG_FILE: &str = "CHANGELOG.md";

/// Transaction directory for an in-flight fold, relative to the repository
/// root. It lives on the same filesystem as `CHANGELOG.md` and `changelog.d/`
/// so every rename into and out of it is atomic. A fixed (non-PID) name lets a
/// later invocation discover an interrupted transaction and recover it.
const TXN_DIR: &str = ".changelog-fold-txn";

/// Journal file inside `TXN_DIR` recording the durable transaction state.
const TXN_JOURNAL: &str = "journal";

/// Staged replacement changelog inside `TXN_DIR`, promoted by an atomic rename.
const TXN_STAGED: &str = "CHANGELOG.md.new";

/// Byte-for-byte backup of the pre-fold changelog inside `TXN_DIR`, used to
/// restore the original on rollback.
const TXN_BACKUP: &str = "CHANGELOG.md.bak";

/// Subdirectory of `TXN_DIR` holding fragments reserved (moved aside) for the
/// in-flight fold.
const TXN_RESERVED: &str = "reserved";

/// Journal marker written and fsynced once the transaction is prepared but
/// before the changelog is promoted. Recovery from this state rolls back.
const STATE_PREPARED: &str = "PREPARED";

/// Journal marker written and fsynced only after the changelog promotion rename
/// has returned. Recovery from this state rolls forward (finishes cleanup).
const STATE_COMMITTED: &str = "COMMITTED";

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

    /// Fold the reasons of `other` into this error, so a primary failure and a
    /// recovery failure that follows it are both surfaced rather than the
    /// recovery outcome being silently discarded.
    fn chained(mut self, other: FoldError) -> Self {
        self.0.extend(other.0);
        self
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
    // A prior fold may have been interrupted mid-transaction, leaving a durable
    // journal. Resolve it before reading fragments so a rolled-back transaction
    // restores its reserved fragments and a committed one is finished. Recovery
    // mutates the tree, so it only runs in the mutating mode; `--check` stays
    // read-only.
    if mode == Mode::Apply {
        recover(repo_root)?;
    }

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

    apply_fold(
        repo_root,
        &changelog_path,
        &changelog,
        &folded,
        &outcome.folded,
    )?;

    Ok(outcome)
}

/// Boundary at which a test may simulate an abrupt process death, leaving the
/// transaction journal on disk exactly as a real crash would.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum FoldStage {
    /// Journal is durably `PREPARED`; no fragment reserved, changelog untouched.
    AfterPrepare,
    /// The `usize`-th fragment (0-based) has just been reserved.
    AfterReserve(usize),
    /// Every fragment is reserved; the changelog is not yet promoted.
    AfterReserveAll,
    /// The changelog promotion rename has returned and is durable, but the
    /// `COMMITTED` journal write has not happened yet. Recovery from here still
    /// rolls back: the linearization point is the journal write, not the
    /// promotion, so a crash in this window undoes the visible promotion.
    AfterPromoteBeforeCommit,
    /// The changelog promotion rename and the `COMMITTED` journal write have
    /// both returned; only cleanup remains.
    AfterCommit,
}

/// Boundary inside [`recover`] at which a test may simulate an abrupt process
/// death, so recovery itself can be interrupted and then re-run to prove
/// repeated recovery still folds each entry exactly once.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum RecoverStage {
    /// Forward cleanup: before the reserved fragments are removed.
    ForwardBeforeReserved,
    /// Forward cleanup: reserved fragments gone, before the backup is removed.
    ForwardBeforeBackup,
    /// Forward cleanup: only the journal remains, before it is unlinked. While
    /// the journal survives, a re-run still rolls forward.
    ForwardBeforeJournal,
    /// Forward cleanup: the journal is gone, before the now-empty transaction
    /// directory is removed.
    ForwardBeforeRmdir,
    /// Rollback: fragments restored, before the backup is renamed back over the
    /// changelog.
    RollbackBeforeBackup,
    /// Rollback: changelog restored, before the transaction directory is
    /// removed.
    RollbackBeforeRmdir,
}

/// A test hook's decision at a fold or recovery boundary.
#[cfg(test)]
enum HookOutcome {
    Continue,
    /// Return immediately without rollback or cleanup, as if the process had
    /// died. The journal and any reserved state stay on disk for recovery.
    Crash,
}

/// fsync a directory at `path` so a rename/create/unlink inside it is durable.
fn sync_dir(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Remove a file at `path`, treating an already-absent file as success so a
/// re-run of cleanup after an interruption is idempotent.
fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Remove a directory tree at `path`, treating an already-absent tree as
/// success so a re-run of cleanup after an interruption is idempotent.
fn remove_dir_all_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Write `data` to `path`, truncating, and fsync it before returning.
fn write_sync(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// Durable transaction journal contents.
struct Journal {
    state: String,
}

/// Write the journal marker `state` into `txn` and fsync both the file and the
/// transaction directory so the state transition is crash-durable.
fn write_journal(txn: &Path, state: &str) -> std::io::Result<()> {
    write_sync(
        &txn.join(TXN_JOURNAL),
        format!("state={state}\n").as_bytes(),
    )?;
    sync_dir(txn)
}

/// Read the journal from `txn`, if present and well-formed.
fn read_journal(txn: &Path) -> Option<Journal> {
    let text = fs::read_to_string(txn.join(TXN_JOURNAL)).ok()?;
    let state = text
        .lines()
        .find_map(|line| line.strip_prefix("state="))?
        .to_string();
    Some(Journal { state })
}

/// Resolve any transaction left on disk by an interrupted fold.
///
/// The `COMMITTED` marker is written and fsynced only after the changelog
/// promotion rename returns, so its presence proves the promotion happened:
/// recovery finishes forward by discarding the transaction (the reserved
/// fragments are already consumed). Any earlier state - or an unreadable
/// journal - means the promotion did not durably happen, so recovery rolls
/// back: reserved fragments return to `changelog.d/` and the original changelog
/// is restored from its backup. Either way the tree ends fully folded or fully
/// unfolded, never half-consumed.
///
/// Recovery is itself idempotent: interrupting it and re-running leaves the
/// same fully-folded or fully-unfolded tree, because forward cleanup removes
/// the `COMMITTED` journal last (so a re-run still classifies as forward until
/// nothing restorable remains) and every unlink tolerates an already-absent
/// target.
fn recover(repo_root: &Path) -> Result<(), FoldError> {
    recover_hooked(
        repo_root,
        #[cfg(test)]
        &mut |_| HookOutcome::Continue,
    )
}

fn recover_hooked(
    repo_root: &Path,
    #[cfg(test)] hook: &mut dyn FnMut(RecoverStage) -> HookOutcome,
) -> Result<(), FoldError> {
    let txn = repo_root.join(TXN_DIR);
    if !txn.exists() {
        return Ok(());
    }

    let committed = read_journal(&txn)
        .map(|journal| journal.state == STATE_COMMITTED)
        .unwrap_or(false);

    if committed {
        finish_forward(
            repo_root,
            &txn,
            #[cfg(test)]
            hook,
        )
    } else {
        roll_back(
            repo_root,
            &txn,
            #[cfg(test)]
            hook,
        )
    }
}

/// Discard a committed transaction: the promotion already happened, so only the
/// consumed fragments and staging state remain to be cleared.
///
/// Cleanup runs journal-last: the reserved fragments, the staged rewrite, and
/// the backup are removed and fsynced first, and only then is the `COMMITTED`
/// journal unlinked, followed by the now-empty transaction directory. While the
/// journal survives, an interrupted re-run still classifies the transaction as
/// committed and re-enters this forward path idempotently; once the journal is
/// gone the only residue is an empty directory, which a re-run removes via the
/// (restore-free, since nothing restorable remains) rollback path. This is what
/// makes `remove_dir_all` unsafe here: it could unlink the journal, backup, and
/// reserved fragments in any order, so an interruption could drop the
/// `COMMITTED` marker while a restorable backup and reserved fragments survived,
/// and the next recovery would roll a promoted fold back and re-fold it -
/// duplicating or losing entries.
fn finish_forward(
    repo_root: &Path,
    txn: &Path,
    #[cfg(test)] hook: &mut dyn FnMut(RecoverStage) -> HookOutcome,
) -> Result<(), FoldError> {
    macro_rules! crash_if_hooked {
        ($stage:expr) => {
            #[cfg(test)]
            if let HookOutcome::Crash = hook($stage) {
                return Err(FoldError::single("simulated crash during forward recovery"));
            }
        };
    }

    crash_if_hooked!(RecoverStage::ForwardBeforeReserved);
    remove_dir_all_if_exists(&txn.join(TXN_RESERVED)).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}/{TXN_RESERVED}: cannot clear reserved fragments: {err}"
        ))
    })?;
    remove_file_if_exists(&txn.join(TXN_STAGED)).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}/{TXN_STAGED}: cannot clear staging: {err}"
        ))
    })?;
    sync_dir(txn).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot durably clear staging state: {err}"
        ))
    })?;

    crash_if_hooked!(RecoverStage::ForwardBeforeBackup);
    remove_file_if_exists(&txn.join(TXN_BACKUP)).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}/{TXN_BACKUP}: cannot clear backup: {err}"
        ))
    })?;
    sync_dir(txn).map_err(|err| {
        FoldError::single(format!("{TXN_DIR}: cannot durably clear backup: {err}"))
    })?;

    // The journal is the last thing removed: until this returns, a crashed
    // re-run still sees COMMITTED and re-enters this idempotent forward path.
    crash_if_hooked!(RecoverStage::ForwardBeforeJournal);
    remove_file_if_exists(&txn.join(TXN_JOURNAL)).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}/{TXN_JOURNAL}: cannot clear journal: {err}"
        ))
    })?;
    sync_dir(txn).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot durably clear committed journal: {err}"
        ))
    })?;

    crash_if_hooked!(RecoverStage::ForwardBeforeRmdir);
    remove_dir_all_if_exists(txn).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot finish committed fold recovery: {err}"
        ))
    })?;
    sync_dir(repo_root).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot durably remove transaction: {err}"
        ))
    })
}

/// Undo an uncommitted transaction: return every reserved fragment to
/// `changelog.d/` and restore the original changelog from its backup, then
/// remove the transaction directory. Restorative steps run before the backup is
/// consumed so a crash mid-rollback stays recoverable on the next pass. Errors
/// are surfaced, never swallowed.
fn roll_back(
    repo_root: &Path,
    txn: &Path,
    #[cfg(test)] hook: &mut dyn FnMut(RecoverStage) -> HookOutcome,
) -> Result<(), FoldError> {
    macro_rules! crash_if_hooked {
        ($stage:expr) => {
            #[cfg(test)]
            if let HookOutcome::Crash = hook($stage) {
                return Err(FoldError::single(
                    "simulated crash during rollback recovery",
                ));
            }
        };
    }

    let fragment_dir = repo_root.join(FRAGMENT_DIR);
    let reserved_dir = txn.join(TXN_RESERVED);
    if reserved_dir.exists() {
        let entries = fs::read_dir(&reserved_dir).map_err(|err| {
            FoldError::single(format!("{TXN_DIR}/{TXN_RESERVED}: cannot read: {err}"))
        })?;
        for entry in entries {
            let entry = entry.map_err(|err| {
                FoldError::single(format!(
                    "{TXN_DIR}/{TXN_RESERVED}: cannot read entry: {err}"
                ))
            })?;
            let name = entry.file_name();
            let canonical = fragment_dir.join(&name);
            fs::rename(entry.path(), &canonical).map_err(|err| {
                FoldError::single(format!(
                    "{FRAGMENT_DIR}/{}: cannot restore reserved fragment: {err}",
                    name.to_string_lossy()
                ))
            })?;
        }
        sync_dir(&fragment_dir).map_err(|err| {
            FoldError::single(format!(
                "{FRAGMENT_DIR}: cannot durably restore fragments: {err}"
            ))
        })?;
    }

    crash_if_hooked!(RecoverStage::RollbackBeforeBackup);
    let backup = txn.join(TXN_BACKUP);
    if backup.exists() {
        let changelog_path = repo_root.join(CHANGELOG_FILE);
        // Renaming the backup over the changelog is atomic and, by removing the
        // backup, makes the restore idempotent: a re-run sees no backup and
        // leaves the already-restored changelog alone.
        fs::rename(&backup, &changelog_path).map_err(|err| {
            FoldError::single(format!(
                "{CHANGELOG_FILE}: cannot restore from backup: {err}"
            ))
        })?;
        sync_dir(repo_root).map_err(|err| {
            FoldError::single(format!(
                "{CHANGELOG_FILE}: cannot durably restore backup: {err}"
            ))
        })?;
    }

    crash_if_hooked!(RecoverStage::RollbackBeforeRmdir);
    remove_dir_all_if_exists(txn).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot remove rolled-back transaction: {err}"
        ))
    })?;
    sync_dir(repo_root).map_err(|err| {
        FoldError::single(format!(
            "{TXN_DIR}: cannot durably remove transaction: {err}"
        ))
    })
}

/// Run inline recovery after a mid-fold failure, folding any recovery failure
/// into the primary error so a failed recovery is surfaced rather than
/// silently discarded.
fn recover_and_chain(repo_root: &Path, primary: FoldError) -> FoldError {
    match recover(repo_root) {
        Ok(()) => primary,
        Err(recovery) => primary.chained(recovery),
    }
}

/// Commit a computed fold to disk as a crash-recoverable transaction.
///
/// The changelog rewrite and the fragment removals are made all-or-nothing,
/// and unlike a plain staging directory the transaction survives an abrupt
/// process death: a durable journal plus a byte backup of the original
/// changelog let a later invocation ([`recover`]) either finish a committed
/// fold or roll an uncommitted one all the way back. No interruption can leave
/// a corrupted changelog, duplicated entries on retry, or a half-consumed
/// fragment set. The steps, each made durable before the next:
///
/// 1. **Prepare.** Create `TXN_DIR`, write a byte backup of the current
///    changelog and the staged replacement, and fsync a `PREPARED` journal.
///    Until this is durable the tree is untouched.
/// 2. **Reserve.** Move each consumed fragment into `TXN_DIR/reserved/`,
///    fsyncing the directories after each move. A crash here recovers as a
///    rollback: the reserved fragments return and the changelog is untouched.
/// 3. **Commit.** Promote the staged changelog over `CHANGELOG.md` with one
///    atomic rename, then fsync a `COMMITTED` journal. The journal write is the
///    linearization point: a crash before it rolls back (restoring the original
///    changelog from the backup); a crash after it rolls forward.
/// 4. **Cleanup.** Remove `TXN_DIR`. A crash here is finished by recovery.
fn apply_fold(
    repo_root: &Path,
    changelog_path: &Path,
    original: &str,
    folded: &str,
    folded_names: &[String],
) -> Result<(), FoldError> {
    apply_fold_hooked(
        repo_root,
        changelog_path,
        original,
        folded,
        folded_names,
        #[cfg(test)]
        &mut |_| HookOutcome::Continue,
    )
}

fn apply_fold_hooked(
    repo_root: &Path,
    changelog_path: &Path,
    original: &str,
    folded: &str,
    folded_names: &[String],
    #[cfg(test)] hook: &mut dyn FnMut(FoldStage) -> HookOutcome,
) -> Result<(), FoldError> {
    // A crash simulated by a test hook returns this sentinel without running
    // rollback or cleanup, leaving the transaction on disk for recovery. A real
    // error (below) instead recovers inline before returning.
    macro_rules! crash_if_hooked {
        ($stage:expr) => {
            #[cfg(test)]
            if let HookOutcome::Crash = hook($stage) {
                return Err(FoldError::single("simulated crash"));
            }
        };
    }
    // `original`, `changelog_path`, and (in non-test builds) the hook are all
    // consumed below; nothing to silence.

    let fragment_dir = repo_root.join(FRAGMENT_DIR);
    let txn = repo_root.join(TXN_DIR);

    // Any leftover transaction (from an earlier crash) is resolved before a new
    // one begins, so two transactions never coexist.
    if txn.exists() {
        recover(repo_root)?;
    }

    // --- Prepare -----------------------------------------------------------
    // Helper: on a real I/O error, recover (rolling the just-started
    // transaction back) and surface the original reason.
    let prepare = || -> Result<(), FoldError> {
        fs::create_dir(&txn).map_err(|err| {
            FoldError::single(format!(
                "{TXN_DIR}: cannot create transaction directory: {err}"
            ))
        })?;
        sync_dir(repo_root).map_err(|err| {
            FoldError::single(format!(
                "{TXN_DIR}: cannot durably create transaction: {err}"
            ))
        })?;
        write_sync(&txn.join(TXN_BACKUP), original.as_bytes()).map_err(|err| {
            FoldError::single(format!(
                "{TXN_DIR}/{TXN_BACKUP}: cannot back up changelog: {err}"
            ))
        })?;
        write_sync(&txn.join(TXN_STAGED), folded.as_bytes()).map_err(|err| {
            FoldError::single(format!(
                "{TXN_DIR}/{TXN_STAGED}: cannot stage rewrite: {err}"
            ))
        })?;
        fs::create_dir(txn.join(TXN_RESERVED)).map_err(|err| {
            FoldError::single(format!("{TXN_DIR}/{TXN_RESERVED}: cannot create: {err}"))
        })?;
        write_journal(&txn, STATE_PREPARED).map_err(|err| {
            FoldError::single(format!("{TXN_DIR}/{TXN_JOURNAL}: cannot write: {err}"))
        })?;
        Ok(())
    };
    if let Err(err) = prepare() {
        return Err(recover_and_chain(repo_root, err));
    }
    crash_if_hooked!(FoldStage::AfterPrepare);

    // --- Reserve -----------------------------------------------------------
    // The reservation index is only read by the test crash hook; keep
    // `enumerate` so that hook can name a specific boundary.
    #[allow(clippy::unused_enumerate_index)]
    for (_index, name) in folded_names.iter().enumerate() {
        let canonical = fragment_dir.join(name);
        let aside = txn.join(TXN_RESERVED).join(name);
        if let Err(err) = fs::rename(&canonical, &aside) {
            let err = FoldError::single(format!(
                "{FRAGMENT_DIR}/{name}: cannot reserve for removal: {err}"
            ));
            return Err(recover_and_chain(repo_root, err));
        }
        if let Err(err) = sync_dir(&fragment_dir).and_then(|()| sync_dir(&txn.join(TXN_RESERVED))) {
            let err = FoldError::single(format!(
                "{FRAGMENT_DIR}/{name}: cannot durably reserve: {err}"
            ));
            return Err(recover_and_chain(repo_root, err));
        }
        crash_if_hooked!(FoldStage::AfterReserve(_index));
    }
    crash_if_hooked!(FoldStage::AfterReserveAll);

    // --- Commit ------------------------------------------------------------
    // The atomic rename is the only moment CHANGELOG.md changes; the fsynced
    // COMMITTED journal that follows is the transaction's linearization point.
    if let Err(err) = fs::rename(txn.join(TXN_STAGED), changelog_path) {
        let err = FoldError::single(format!("{CHANGELOG_FILE}: cannot promote rewrite: {err}"));
        return Err(recover_and_chain(repo_root, err));
    }
    if let Err(err) = sync_dir(repo_root) {
        let err = FoldError::single(format!("{CHANGELOG_FILE}: cannot durably promote: {err}"));
        return Err(recover_and_chain(repo_root, err));
    }
    // A crash here - promotion durable, COMMITTED not yet written - must roll
    // back on recovery, undoing the visible promotion, because the journal
    // write below is the linearization point.
    crash_if_hooked!(FoldStage::AfterPromoteBeforeCommit);
    if let Err(err) = write_journal(&txn, STATE_COMMITTED) {
        // The promotion is already durable but the commit marker is not. Rather
        // than risk a rollback that would undo a visible changelog change,
        // surface the failure; a re-run's recovery sees a non-committed journal
        // and rolls back cleanly to the backed-up original.
        let err = FoldError::single(format!(
            "{TXN_DIR}/{TXN_JOURNAL}: cannot commit transaction: {err}"
        ));
        return Err(recover_and_chain(repo_root, err));
    }
    crash_if_hooked!(FoldStage::AfterCommit);

    // --- Cleanup -----------------------------------------------------------
    finish_forward(
        repo_root,
        &txn,
        #[cfg(test)]
        &mut |_| HookOutcome::Continue,
    )
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
    fn fold_repo_leaves_tree_unchanged_when_transaction_cannot_be_created() {
        let repo = TempRepo::new("stage-fail");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
        let before = repo.changelog();

        fs::set_permissions(&repo.root, fs::Permissions::from_mode(0o555)).unwrap();
        // Root bypasses directory permission bits; the injection is a no-op then.
        if fs::create_dir(repo.root.join(".probe-writable")).is_ok() {
            let _ = fs::remove_dir(repo.root.join(".probe-writable"));
            fs::set_permissions(&repo.root, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipping fold_repo transaction-failure test: running as root");
            return;
        }

        let err = fold_repo(&repo.root, Mode::Apply).expect_err("transaction create fails");
        assert!(
            err.reasons()
                .iter()
                .any(|reason| reason.contains("transaction directory")),
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

    /// Compute the fold inputs for `repo` exactly as `fold_repo` would, so a
    /// test can drive `apply_fold_hooked` directly with a crash hook.
    fn compute_fold(repo: &TempRepo) -> (String, String, Vec<String>) {
        let fragments = load_fragments(&repo.root).expect("load fragments");
        let original = fs::read_to_string(repo.root.join(CHANGELOG_FILE)).expect("read changelog");
        let folded = fold_unreleased(&original, &fragments).expect("fold");
        let names = fragments.iter().map(|f| f.name.clone()).collect();
        (original, folded, names)
    }

    /// Drive a fold to `crash_at`, then abandon it exactly as a crashed process
    /// would: the transaction directory is left on disk for recovery.
    fn crash_at_boundary(repo: &TempRepo, crash_at: FoldStage) {
        let (original, folded, names) = compute_fold(repo);
        let changelog_path = repo.root.join(CHANGELOG_FILE);
        let mut fired = false;
        let result = apply_fold_hooked(
            &repo.root,
            &changelog_path,
            &original,
            &folded,
            &names,
            &mut |stage| {
                if stage == crash_at {
                    fired = true;
                    HookOutcome::Crash
                } else {
                    HookOutcome::Continue
                }
            },
        );
        assert!(fired, "crash hook never fired at {crash_at:?}");
        assert!(
            result.is_err(),
            "a simulated crash returns the sentinel error"
        );
        assert!(
            repo.root.join(TXN_DIR).exists(),
            "the crash leaves the transaction on disk for recovery"
        );
    }

    /// Crash at `crash_at`, then prove the tree recovers with every entry folded
    /// exactly once (no data loss, no double-fold) and no residual transaction.
    /// `pre_promoted` says whether the crash left `CHANGELOG.md` already
    /// rewritten; `committed` says whether the crash landed after the
    /// linearization point (the `COMMITTED` journal write), so recovery rolls
    /// the fold forward rather than back.
    fn assert_recovers(tag: &str, crash_at: FoldStage, pre_promoted: bool, committed: bool) {
        let repo = TempRepo::new(tag);
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
        repo.write_fragment("feature-b.md", "### Fixed\n\n- fix from b\n");

        crash_at_boundary(&repo, crash_at);

        if pre_promoted {
            assert!(
                repo.changelog().contains("- from a\n"),
                "the crash left the changelog already promoted"
            );
        } else {
            assert_eq!(
                repo.changelog(),
                CHANGELOG,
                "a pre-promotion crash leaves the changelog untouched"
            );
        }

        // A fresh invocation recovers the interrupted transaction before doing
        // anything else, then folds whatever remains.
        let outcome = fold_repo(&repo.root, Mode::Apply).expect("recover then fold");

        let folded = repo.changelog();
        assert_eq!(
            folded.matches("- from a\n").count(),
            1,
            "entry a appears exactly once: {folded}"
        );
        assert_eq!(
            folded.matches("- fix from b\n").count(),
            1,
            "entry b appears exactly once: {folded}"
        );
        assert!(repo.fragment_names().is_empty(), "every fragment consumed");
        assert!(
            !repo.root.join(TXN_DIR).exists(),
            "recovery clears the transaction directory"
        );

        if committed {
            assert!(
                outcome.folded.is_empty(),
                "an after-commit crash is finished by recovery, so the fold is a no-op"
            );
        } else {
            assert_eq!(
                outcome.folded.len(),
                2,
                "a before-commit crash rolls back, so both fragments re-fold"
            );
        }
    }

    #[test]
    fn recovers_after_crash_following_prepare() {
        assert_recovers("crash-prepare", FoldStage::AfterPrepare, false, false);
    }

    #[test]
    fn recovers_after_crash_following_first_reservation() {
        assert_recovers("crash-reserve-0", FoldStage::AfterReserve(0), false, false);
    }

    #[test]
    fn recovers_after_crash_following_last_reservation() {
        assert_recovers("crash-reserve-1", FoldStage::AfterReserve(1), false, false);
    }

    #[test]
    fn recovers_after_crash_following_all_reservations() {
        assert_recovers(
            "crash-reserve-all",
            FoldStage::AfterReserveAll,
            false,
            false,
        );
    }

    #[test]
    fn recovers_after_crash_between_promote_and_commit() {
        // The changelog is already rewritten on disk, but the COMMITTED journal
        // write has not happened, so recovery must roll the visible promotion
        // back and both fragments must re-fold.
        assert_recovers(
            "crash-promote-precommit",
            FoldStage::AfterPromoteBeforeCommit,
            true,
            false,
        );
    }

    #[test]
    fn recovers_after_crash_following_commit() {
        assert_recovers("crash-commit", FoldStage::AfterCommit, true, true);
    }

    #[test]
    fn recover_is_a_no_op_without_a_transaction() {
        let repo = TempRepo::new("recover-noop");
        repo.write_changelog(CHANGELOG);
        recover(&repo.root).expect("no transaction to recover");
        assert_eq!(repo.changelog(), CHANGELOG, "changelog untouched");
        assert!(!repo.root.join(TXN_DIR).exists(), "no transaction created");
    }

    #[test]
    fn recovery_rolls_forward_only_a_committed_journal() {
        // A transaction whose journal never reached COMMITTED must roll back
        // even though its reserved fragments were moved aside, restoring the
        // original changelog from the backup.
        let repo = TempRepo::new("recover-rollback");
        repo.write_changelog(CHANGELOG);
        repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");

        crash_at_boundary(&repo, FoldStage::AfterReserve(0));
        // The reserved fragment is out of changelog.d/ and inside the txn.
        assert!(
            repo.fragment_names().is_empty(),
            "fragment reserved, not in place"
        );
        let txn = repo.root.join(TXN_DIR);
        assert!(txn.join(TXN_RESERVED).join("feature-a.md").exists());

        recover(&repo.root).expect("rollback recovery");
        assert_eq!(repo.changelog(), CHANGELOG, "original changelog restored");
        assert_eq!(
            repo.fragment_names(),
            vec!["feature-a.md".to_string()],
            "reserved fragment restored"
        );
        assert!(!txn.exists(), "transaction cleared");
    }

    /// Crash recovery once at `target`, assert the interrupted recovery both
    /// errored and left the transaction on disk, then drive recovery to
    /// completion twice to prove re-running it is idempotent.
    fn interrupt_recovery_at(repo: &TempRepo, target: RecoverStage) {
        let mut fired = false;
        let result = recover_hooked(&repo.root, &mut |stage| {
            if stage == target {
                fired = true;
                HookOutcome::Crash
            } else {
                HookOutcome::Continue
            }
        });
        assert!(fired, "recovery crash hook never fired at {target:?}");
        assert!(
            result.is_err(),
            "{target:?}: a simulated recovery crash surfaces an error"
        );
        assert!(
            repo.root.join(TXN_DIR).exists(),
            "{target:?}: the interrupted recovery leaves the transaction on disk"
        );
        recover(&repo.root).expect("re-running recovery completes the interrupted work");
        recover(&repo.root).expect("a second recovery pass is a no-op");
    }

    #[test]
    fn forward_recovery_is_idempotent_under_interruption() {
        // A committed transaction whose forward cleanup is interrupted at any
        // journal-last step still converges - on repeated recovery - to the
        // fold applied exactly once, never rolled back.
        for target in [
            RecoverStage::ForwardBeforeReserved,
            RecoverStage::ForwardBeforeBackup,
            RecoverStage::ForwardBeforeJournal,
            RecoverStage::ForwardBeforeRmdir,
        ] {
            let repo = TempRepo::new(&format!("fwd-{target:?}"));
            repo.write_changelog(CHANGELOG);
            repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
            repo.write_fragment("feature-b.md", "### Fixed\n\n- fix from b\n");

            // Leave a committed transaction on disk (changelog already promoted).
            crash_at_boundary(&repo, FoldStage::AfterCommit);
            assert!(
                repo.changelog().contains("- from a\n"),
                "{target:?}: the committed crash promoted the changelog"
            );

            interrupt_recovery_at(&repo, target);

            let folded = repo.changelog();
            assert_eq!(
                folded.matches("- from a\n").count(),
                1,
                "{target:?}: entry a folded exactly once: {folded}"
            );
            assert_eq!(
                folded.matches("- fix from b\n").count(),
                1,
                "{target:?}: entry b folded exactly once: {folded}"
            );
            assert!(
                repo.fragment_names().is_empty(),
                "{target:?}: every fragment stays consumed"
            );
            assert!(
                !repo.root.join(TXN_DIR).exists(),
                "{target:?}: recovery clears the transaction"
            );
        }
    }

    #[test]
    fn rollback_recovery_is_idempotent_under_interruption() {
        // An uncommitted transaction whose rollback is interrupted still
        // converges - on repeated recovery - to the original changelog with
        // every fragment restored, and a fresh fold then applies each once.
        for target in [
            RecoverStage::RollbackBeforeBackup,
            RecoverStage::RollbackBeforeRmdir,
        ] {
            let repo = TempRepo::new(&format!("rb-{target:?}"));
            repo.write_changelog(CHANGELOG);
            repo.write_fragment("feature-a.md", "### Added\n\n- from a\n");
            repo.write_fragment("feature-b.md", "### Fixed\n\n- fix from b\n");

            // Leave an uncommitted transaction on disk (fragments reserved,
            // changelog untouched).
            crash_at_boundary(&repo, FoldStage::AfterReserveAll);
            assert_eq!(
                repo.changelog(),
                CHANGELOG,
                "{target:?}: an uncommitted crash leaves the changelog untouched"
            );

            interrupt_recovery_at(&repo, target);

            assert_eq!(
                repo.changelog(),
                CHANGELOG,
                "{target:?}: rollback restores the original changelog"
            );
            assert!(
                !repo.root.join(TXN_DIR).exists(),
                "{target:?}: recovery clears the transaction"
            );

            // The rolled-back fragments are back in place and re-fold cleanly.
            let outcome = fold_repo(&repo.root, Mode::Apply).expect("re-fold after rollback");
            assert_eq!(
                outcome.folded.len(),
                2,
                "{target:?}: both restored fragments re-fold"
            );
            let folded = repo.changelog();
            assert_eq!(
                folded.matches("- from a\n").count(),
                1,
                "{target:?}: entry a folded exactly once after rollback: {folded}"
            );
            assert_eq!(
                folded.matches("- fix from b\n").count(),
                1,
                "{target:?}: entry b folded exactly once after rollback: {folded}"
            );
        }
    }
}
