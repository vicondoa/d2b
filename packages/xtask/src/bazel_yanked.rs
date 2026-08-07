#![allow(dead_code)]

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const LOCKS: &[&str] = &["packages/Cargo.lock"];
const SNAPSHOT: &str = "bazel/supply_chain/yanked-snapshot.json";
pub const YANKED_PREVIEW: &str = ".scratch/bazel/yanked-snapshot.json";
pub const YANKED_DRIFT_REMEDIATION: &str = "\
D2B-BZLDRIFT-YANKED: yanked snapshot is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-yanked-refresh
Review and commit bazel/supply_chain/yanked-snapshot.json.
Rerun cargo xtask bazel-yanked-check, then rerun the failed command.";

/// The injected boundary for the one reviewed, networked yanked-index refresh.
#[allow(dead_code)]
pub trait YankedIndex {
    type Error: Error + Send + Sync + 'static;

    fn revision(&mut self) -> Result<String, Self::Error>;
    fn is_yanked(&mut self, name: &str, version: &str) -> Result<bool, Self::Error>;
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IndexClientError {
    message: String,
}

impl IndexClientError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for IndexClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for IndexClientError {}

/// The sole networked implementation of `YankedIndex`.
pub struct IndexClient {
    base_url: String,
}

impl IndexClient {
    pub fn new() -> Result<Self, IndexClientError> {
        let base_url = env::var("D2B_BAZEL_YANKED_INDEX_URL")
            .unwrap_or_else(|_| "https://index.crates.io".to_owned())
            .trim_end_matches('/')
            .to_owned();
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err(IndexClientError::new(
                "yanked index URL must use http or https",
            ));
        }
        Ok(Self { base_url })
    }

    fn fetch(&self, path: &str, head: bool) -> Result<Vec<u8>, IndexClientError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let mut command = Command::new("curl");
        command.args(["--fail", "--silent", "--show-error"]);
        if head {
            command.arg("--head");
        }
        let output = command.arg(url).output().map_err(|error| {
            IndexClientError::new(format!("could not start the index transport: {error}"))
        })?;
        if !output.status.success() {
            return Err(IndexClientError::new(format!(
                "index transport failed with status {}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_owned())
            )));
        }
        Ok(output.stdout)
    }
}

impl YankedIndex for IndexClient {
    type Error = IndexClientError;

    fn revision(&mut self) -> Result<String, Self::Error> {
        if let Some(revision) = env::var_os("D2B_BAZEL_YANKED_INDEX_REVISION") {
            let revision = revision.to_string_lossy().into_owned();
            if !revision.is_empty() {
                return Ok(revision);
            }
        }
        let headers = self.fetch("", true)?;
        let text = String::from_utf8(headers)
            .map_err(|_| IndexClientError::new("index response headers are not UTF-8"))?;
        text.lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("etag") {
                    let value = value.trim();
                    if !value.is_empty() {
                        return Some(value.trim_matches('"').to_owned());
                    }
                }
                None
            })
            .filter(|revision| !revision.is_empty())
            .ok_or_else(|| IndexClientError::new("index response did not carry a revision"))
    }

    fn is_yanked(&mut self, name: &str, version: &str) -> Result<bool, Self::Error> {
        if !valid_crate_name(name) || version.is_empty() || version.contains('/') {
            return Err(IndexClientError::new(
                "index key is not a valid crate/version pair",
            ));
        }
        let path = sparse_index_path(name);
        let payload = self.fetch(&path, false)?;
        let text = String::from_utf8(payload)
            .map_err(|_| IndexClientError::new("index response is not UTF-8"))?;
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).map_err(|error| {
                IndexClientError::new(format!("malformed index payload: {error}"))
            })?;
            if value.get("vers").and_then(Value::as_str) == Some(version) {
                return value.get("yanked").and_then(Value::as_bool).ok_or_else(|| {
                    IndexClientError::new("index entry has no boolean yanked state")
                });
            }
        }
        Err(IndexClientError::new(format!(
            "index response omitted crate version {name} {version}"
        )))
    }
}

pub(crate) fn bazel_yanked_refresh(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if !args.is_empty() {
        return Err("usage: bazel-yanked-refresh".into());
    }
    let root = repo_root()?;
    let mut client = IndexClient::new()?;
    refresh_with_index_at(&root, &root.join(YANKED_PREVIEW), &mut client)
}

pub(crate) fn bazel_yanked_check(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if !args.is_empty() {
        return Err("usage: bazel-yanked-check".into());
    }
    let root = repo_root()?;
    check_snapshot(&root, &root.join(SNAPSHOT))?;
    Ok(Vec::new())
}

pub(crate) fn refresh_with_index<I: YankedIndex>(
    root: &Path,
    index: &mut I,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    refresh_with_index_at(root, &root.join(SNAPSHOT), index)
}

pub(crate) fn refresh_with_index_at<I: YankedIndex>(
    root: &Path,
    output_path: &Path,
    index: &mut I,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let keys = lock_keys(root)?;
    if keys.is_empty() {
        return Err("the committed Cargo locks contain no package keys".into());
    }
    let revision = index.revision().map_err(index_error)?;
    if revision.trim().is_empty() {
        return Err("the yanked index returned no revision".into());
    }

    let mut entries = Vec::with_capacity(keys.len());
    for (name, version) in keys {
        let yanked = index
            .is_yanked(&name, &version)
            .map_err(|error| format!("yanked index lookup failed: {error}"))?;
        entries.push(json!({
            "name": name,
            "version": version,
            "yanked": yanked,
        }));
    }
    let document = json!({
        "indexRevision": revision,
        "entries": entries,
    });
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_vec_pretty(&document)?;
    let mut contents = contents;
    contents.push(b'\n');
    fs::write(output_path, contents)?;
    let relative = output_path
        .strip_prefix(root)
        .unwrap_or(output_path)
        .to_path_buf();
    Ok(vec![relative])
}

pub(crate) fn check_snapshot(root: &Path, snapshot_path: &Path) -> Result<(), Box<dyn Error>> {
    let text = fs::read_to_string(snapshot_path).map_err(|error| {
        let _ = error;
        YANKED_DRIFT_REMEDIATION.to_owned()
    })?;
    let document: Value = serde_json::from_str(&text).map_err(|error| {
        let _ = error;
        YANKED_DRIFT_REMEDIATION.to_owned()
    })?;
    if document
        .get("indexRevision")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(YANKED_DRIFT_REMEDIATION.into());
    }
    let entries = document
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| YANKED_DRIFT_REMEDIATION.to_owned())?;
    let expected = lock_keys(root)?;
    let mut actual = BTreeSet::new();
    let mut previous: Option<(String, String)> = None;
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| YANKED_DRIFT_REMEDIATION.to_owned())?;
        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| YANKED_DRIFT_REMEDIATION.to_owned())?;
        if entry.get("yanked").and_then(Value::as_bool).is_none() {
            return Err(YANKED_DRIFT_REMEDIATION.into());
        }
        let key = (name.to_owned(), version.to_owned());
        if previous.as_ref().is_some_and(|prior| prior >= &key) {
            return Err(YANKED_DRIFT_REMEDIATION.into());
        }
        previous = Some(key.clone());
        if !actual.insert(key) {
            return Err(YANKED_DRIFT_REMEDIATION.into());
        }
    }
    if actual != expected {
        return Err(YANKED_DRIFT_REMEDIATION.into());
    }
    Ok(())
}

pub(crate) fn check_projection(
    snapshot: &Value,
    expected: &BTreeSet<(String, String)>,
) -> Result<(), Box<dyn Error>> {
    let entries = snapshot
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "yanked snapshot has no entries".to_owned())?;
    let actual = entries
        .iter()
        .map(|entry| {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "yanked snapshot entry has no name".to_owned())?;
            let version = entry
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| "yanked snapshot entry has no version".to_owned())?;
            Ok((name.to_owned(), version.to_owned()))
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    if &actual != expected {
        return Err("yanked projection does not match the product snapshot".into());
    }
    Ok(())
}

fn index_error<E: Error + Send + Sync + 'static>(error: E) -> Box<dyn Error> {
    Box::new(error)
}

fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(root) = env::var_os("D2B_BAZEL_WORKTREE") {
        return fs::canonicalize(root)
            .map_err(|error| format!("cannot canonicalize D2B_BAZEL_WORKTREE: {error}").into());
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate repository root".into())
}

fn lock_keys(root: &Path) -> Result<BTreeSet<(String, String)>, Box<dyn Error>> {
    let mut keys = BTreeSet::new();
    for relative in LOCKS {
        let text = fs::read_to_string(root.join(relative))
            .map_err(|error| format!("cannot read committed lock {relative}: {error}"))?;
        for block in text.split("[[package]]").skip(1) {
            let Some(name) = toml_string(block, "name") else {
                continue;
            };
            let Some(version) = toml_string(block, "version") else {
                continue;
            };
            keys.insert((name, version));
        }
    }
    Ok(keys)
}

fn toml_string(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim().strip_prefix('"')?;
        Some(value[..value.find('"')?].to_owned())
    })
}

fn valid_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn sparse_index_path(name: &str) -> String {
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

#[allow(dead_code)]
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Eq, PartialEq)]
    struct FakeError(String);

    impl fmt::Display for FakeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl Error for FakeError {}

    #[derive(Clone, Debug)]
    struct FakeIndex {
        revision: Result<String, FakeError>,
        states: Arc<Mutex<BTreeSet<(String, String)>>>,
        fail_on: Option<(String, String)>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl FakeIndex {
        fn clear() -> Self {
            Self {
                revision: Ok("test-revision".to_owned()),
                states: Arc::new(Mutex::new(BTreeSet::new())),
                fail_on: None,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl YankedIndex for FakeIndex {
        type Error = FakeError;

        fn revision(&mut self) -> Result<String, Self::Error> {
            self.revision.clone()
        }

        fn is_yanked(&mut self, name: &str, version: &str) -> Result<bool, Self::Error> {
            let key = (name.to_owned(), version.to_owned());
            self.calls.lock().unwrap().push(key.clone());
            if self.fail_on.as_ref() == Some(&key) {
                return Err(FakeError("transport failed part-way through".to_owned()));
            }
            Ok(self.states.lock().unwrap().contains(&key))
        }
    }

    fn fixture_root(label: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join(".scratch")
            .join(format!("bazel-yanked-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("packages")).unwrap();
        for lock in LOCKS {
            let path = root.join(lock);
            let body = "[[package]]\nname = \"example\"\nversion = \"1.0.0\"\n";
            fs::write(path, body).unwrap();
        }
        root
    }

    #[test]
    fn refresh_uses_only_the_fake_and_writes_one_deterministic_snapshot() {
        let root = fixture_root("all-clear");
        let mut fake = FakeIndex::clear();
        let output = refresh_with_index(&root, &mut fake).unwrap();
        assert_eq!(output, vec![PathBuf::from(SNAPSHOT)]);
        let snapshot: Value =
            serde_json::from_str(&fs::read_to_string(root.join(SNAPSHOT)).unwrap()).unwrap();
        assert_eq!(snapshot["indexRevision"], "test-revision");
        assert_eq!(snapshot["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            &[("example".to_owned(), "1.0.0".to_owned())]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refresh_records_a_yanked_version() {
        let root = fixture_root("yanked");
        let mut fake = FakeIndex::clear();
        fake.states
            .lock()
            .unwrap()
            .insert(("example".to_owned(), "1.0.0".to_owned()));
        refresh_with_index(&root, &mut fake).unwrap();
        let snapshot: Value =
            serde_json::from_str(&fs::read_to_string(root.join(SNAPSHOT)).unwrap()).unwrap();
        assert_eq!(snapshot["entries"][0]["yanked"], true);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn offline_check_uses_only_the_product_lock_and_rejects_key_mutations() {
        let root = fixture_root("offline-check");
        let mut fake = FakeIndex::clear();
        refresh_with_index(&root, &mut fake).expect("snapshot");
        check_snapshot(&root, &root.join(SNAPSHOT)).expect("matching snapshot");

        let path = root.join(SNAPSHOT);
        let mut document: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        document["entries"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "not-in-product", "version": "1.0.0", "yanked": false}));
        fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let error = check_snapshot(&root, &path).expect_err("extra key must refuse");
        assert!(error.to_string().starts_with("D2B-BZLDRIFT-YANKED:"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refresh_preserves_fake_failures_for_missing_malformed_and_transport_answers() {
        let root = fixture_root("failure");
        let mut transport = FakeIndex::clear();
        transport.fail_on = Some(("example".to_owned(), "1.0.0".to_owned()));
        assert!(
            refresh_with_index(&root, &mut transport)
                .unwrap_err()
                .to_string()
                .contains("transport failed part-way through")
        );

        let mut no_revision = FakeIndex::clear();
        no_revision.revision = Ok(String::new());
        assert!(
            refresh_with_index(&root, &mut no_revision)
                .unwrap_err()
                .to_string()
                .contains("no revision")
        );

        let mut malformed = FakeIndex::clear();
        malformed.revision = Err(FakeError("malformed payload".to_owned()));
        assert!(
            refresh_with_index(&root, &mut malformed)
                .unwrap_err()
                .to_string()
                .contains("malformed payload")
        );
        assert!(!root.join(SNAPSHOT).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tests_do_not_construct_or_reach_the_network_client() {
        let source = include_str!("bazel_yanked.rs");
        let test_source = source.split("#[cfg(test)]").nth(1).unwrap_or_default();
        assert!(!test_source.contains("IndexClient"));
        assert!(!test_source.contains("TcpStream"));
        assert!(!test_source.contains("UdpSocket"));
    }

    #[test]
    fn sparse_index_paths_are_deterministic() {
        assert_eq!(sparse_index_path("a"), "1/a");
        assert_eq!(sparse_index_path("ab"), "2/ab");
        assert_eq!(sparse_index_path("abc"), "3/a/abc");
        assert_eq!(sparse_index_path("abcd"), "ab/cd/abcd");
    }
}
