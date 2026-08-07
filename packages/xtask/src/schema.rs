#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use schemars::schema::RootSchema;

const OUT_DIR_FLAG: &str = "--out-dir";
const SCHEMA_VERSION: &str = "v2";
const SCHEMA_PREVIEW_ROOT: &str = ".scratch/bazel/schema-preview";

pub(crate) fn gen_schemas_with_args(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let explicit_output = parse_gen_schemas_args(args)?;
    let repo_root = super::repo_root()?;
    let output_dir = match explicit_output.as_ref() {
        Some(path) => {
            let path = resolve_output_dir(path.to_owned())?;
            let scratch = repo_root.join(".scratch");
            if !path.starts_with(&scratch) {
                return Err("schema previews must be emitted under .scratch/".into());
            }
            path
        }
        None => repo_root.join(SCHEMA_PREVIEW_ROOT).join(SCHEMA_VERSION),
    };

    ensure_output_directory(&output_dir)?;
    let definitions = schema_definitions();
    let emitted = write_schema_files(&output_dir, &definitions)?;
    if explicit_output.is_some() {
        validate_exact_schema_census(&output_dir, &emitted)?;
    }
    Ok(emitted)
}

fn parse_gen_schemas_args(args: &[String]) -> Result<Option<PathBuf>, Box<dyn Error>> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == OUT_DIR_FLAG && !path.is_empty() => Ok(Some(PathBuf::from(path))),
        _ => Err("usage: gen-schemas [--out-dir <path>]".into()),
    }
}

fn resolve_output_dir(path: PathBuf) -> Result<PathBuf, Box<dyn Error>> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn ensure_output_directory(output_dir: &Path) -> Result<(), Box<dyn Error>> {
    if output_dir.exists() && !output_dir.is_dir() {
        return Err(format!(
            "schema output directory is not a directory: {}",
            output_dir.display()
        )
        .into());
    }
    fs::create_dir_all(output_dir)?;
    Ok(())
}

fn schema_definitions() -> Vec<(&'static str, RootSchema)> {
    vec![
        (
            "allocator.json",
            schemars::schema_for!(d2b_core::allocator_config::AllocatorJson),
        ),
        (
            "bundle.json",
            schemars::schema_for!(d2b_core::bundle::Bundle),
        ),
        (
            "realm-workloads-launcher-v2.json",
            schemars::schema_for!(d2b_core::realm_workloads_launcher::RealmWorkloadsLauncherV2Json),
        ),
        (
            "unsafe-local-workloads.json",
            schemars::schema_for!(d2b_core::unsafe_local_workloads::UnsafeLocalWorkloadsJson),
        ),
        (
            "unsafe-local-helper-wire.json",
            schemars::schema_for!(d2b_contracts::unsafe_local_wire::UnsafeLocalHelperWireSchema),
        ),
        (
            "d2b-realm-core.json",
            schemars::schema_for!(super::D2bRealmCoreSchema),
        ),
        ("host.json", schemars::schema_for!(d2b_core::host::HostJson)),
        (
            "processes.json",
            schemars::schema_for!(d2b_core::processes::ProcessesJson),
        ),
        (
            "storage.json",
            schemars::schema_for!(d2b_core::storage::StorageJson),
        ),
        ("sync.json", schemars::schema_for!(d2b_core::sync::SyncJson)),
        (
            "realm-controllers.json",
            schemars::schema_for!(d2b_core::realm_controller_config::RealmControllersJson),
        ),
        (
            "realm-identity.json",
            schemars::schema_for!(d2b_realm_core::RealmIdentityConfigJson),
        ),
        (
            "storage-lifecycle-report.json",
            schemars::schema_for!(d2b_core::storage_lifecycle::StorageLifecycleReport),
        ),
        (
            "privileges.json",
            schemars::schema_for!(d2b_core::privileges::PrivilegesJson),
        ),
        (
            "closures.json",
            schemars::schema_for!(d2b_core::closures::ClosureMetadata),
        ),
        (
            "minijail-profile.json",
            schemars::schema_for!(d2b_core::minijail_profile::MinijailProfile),
        ),
        (
            "wire-protocol.json",
            schemars::schema_for!(d2b_contracts::WireProtocolSchema),
        ),
        (
            "guest-control.json",
            schemars::schema_for!(d2b_contracts::guest_wire::GuestControlSchema),
        ),
        (
            "manifest_v04.json",
            schemars::schema_for!(d2b_core::manifest_v04::ManifestV04),
        ),
        (
            "audio-state.json",
            schemars::schema_for!(d2b_core::audio_policy::AudioPolicyState),
        ),
    ]
}

fn write_schema_files(
    output_dir: &Path,
    schemas: &[(&str, RootSchema)],
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    ensure_output_directory(output_dir)?;
    super::write_schemas(output_dir, schemas)
}

fn validate_exact_schema_census(
    output_dir: &Path,
    census: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    if census.is_empty() {
        return Err("schema census must be nonempty".into());
    }
    if !output_dir.is_dir() {
        return Err(format!(
            "schema output directory is not a directory: {}",
            output_dir.display()
        )
        .into());
    }

    let mut expected = BTreeSet::<OsString>::new();
    for path in census {
        if path.parent() != Some(output_dir) {
            return Err(format!(
                "schema census path is outside its output directory: {}",
                path.display()
            )
            .into());
        }
        let name = path
            .file_name()
            .ok_or_else(|| format!("schema census path has no file name: {}", path.display()))?
            .to_os_string();
        if !expected.insert(name) {
            return Err(format!(
                "schema census contains a duplicate path: {}",
                path.display()
            )
            .into());
        }

        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(format!("schema output is not a regular file: {}", path.display()).into());
        }
        let contents = fs::read(path)?;
        if contents.is_empty() {
            return Err(format!("schema output is empty: {}", path.display()).into());
        }
        serde_json::from_slice::<serde_json::Value>(&contents).map_err(|error| {
            format!(
                "schema output is not valid JSON ({}): {error}",
                path.display()
            )
        })?;
    }

    let mut actual = BTreeSet::<OsString>::new();
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if !entry.file_type()?.is_file() {
            return Err(format!(
                "schema output directory contains a non-file entry: {}",
                entry.path().display()
            )
            .into());
        }
        actual.insert(name);
    }
    if actual != expected {
        return Err(format!(
            "schema output census differs: expected {:?}, found {:?}",
            expected, actual
        )
        .into());
    }
    Ok(())
}

pub(crate) fn compare_independent_schema_trees(
    first: &Path,
    second: &Path,
) -> Result<(), Box<dyn Error>> {
    let first_files = schema_files(first)?;
    let second_files = schema_files(second)?;
    if first_files.is_empty() || second_files.is_empty() {
        return Err("independent schema census must be nonempty".into());
    }
    if first_files.keys().collect::<BTreeSet<_>>() != second_files.keys().collect::<BTreeSet<_>>() {
        return Err("independent schema censuses differ".into());
    }
    for name in first_files.keys() {
        let first_contents = fs::read(first.join(name))?;
        let second_contents = fs::read(second.join(name))?;
        serde_json::from_slice::<serde_json::Value>(&first_contents)?;
        serde_json::from_slice::<serde_json::Value>(&second_contents)?;
        if first_contents != second_contents {
            return Err(format!(
                "independent schema output differs: {}",
                name.to_string_lossy()
            )
            .into());
        }
    }
    Ok(())
}

fn schema_files(directory: &Path) -> Result<BTreeMap<OsString, PathBuf>, Box<dyn Error>> {
    if !directory.is_dir() {
        return Err(format!(
            "schema output directory is not a directory: {}",
            directory.display()
        )
        .into());
    }
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(format!(
                "schema output contains a non-file entry: {}",
                entry.path().display()
            )
            .into());
        }
        files.insert(entry.file_name(), entry.path());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        gen_schemas_with_args, parse_gen_schemas_args, validate_exact_schema_census,
        write_schema_files,
    };

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos();
            let path = super::super::repo_root()
                .expect("repository root")
                .join(".scratch")
                .join(format!(
                    "xtask-schema-{label}-{}-{nonce}",
                    std::process::id()
                ));
            fs::create_dir(&path).expect("create isolated schema test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn gen_schemas_out_dir_rejects_invalid_layouts() {
        assert!(parse_gen_schemas_args(&args(&["--out-dir"])).is_err());
        assert!(parse_gen_schemas_args(&args(&["--output", "schemas"])).is_err());
        assert!(parse_gen_schemas_args(&args(&["--out-dir", "one", "--out-dir", "two"])).is_err());

        let temp = TempDir::new("invalid-layout");
        let output_file = temp.path().join("not-a-directory");
        fs::write(&output_file, b"occupied").expect("create invalid output path");
        let error = gen_schemas_with_args(&args(&[
            "--out-dir",
            output_file.to_str().expect("temporary path is UTF-8"),
        ]))
        .expect_err("a regular file cannot be a schema output directory");
        assert!(error.to_string().contains("output directory"));
    }

    #[test]
    fn emitted_census_is_the_manifest_returned_by_the_writer() {
        let temp = TempDir::new("writer-census");
        let definitions = vec![
            (
                "first.json",
                schemars::schema_for!(d2b_core::allocator_config::AllocatorJson),
            ),
            (
                "second.json",
                schemars::schema_for!(d2b_core::bundle::Bundle),
            ),
        ];
        let expected = definitions
            .iter()
            .map(|(name, _)| temp.path().join(name))
            .collect::<Vec<_>>();

        let emitted = write_schema_files(temp.path(), &definitions).expect("write schemas");

        assert_eq!(emitted, expected);
    }

    #[test]
    fn independent_generated_trees_have_exact_nonempty_json_censuses_before_comparison() {
        let temp = TempDir::new("independent-trees");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let first_emitted = gen_schemas_with_args(&args(&["--out-dir", first.to_str().unwrap()]))
            .expect("first generation");
        let second_emitted = gen_schemas_with_args(&args(&["--out-dir", second.to_str().unwrap()]))
            .expect("second generation");

        validate_exact_schema_census(&first, &first_emitted).expect("first census is exact");
        let second_census = first_emitted
            .iter()
            .zip(&second_emitted)
            .map(|(first_path, second_path)| {
                (
                    first_path
                        .file_name()
                        .expect("first emitted path has a file name"),
                    second_path.to_owned(),
                )
            })
            .map(|(name, path)| {
                assert_eq!(path.file_name(), Some(name));
                path
            })
            .collect::<Vec<_>>();
        validate_exact_schema_census(&second, &second_census).expect("second census is exact");

        assert_eq!(
            first_emitted
                .iter()
                .map(|path| path.file_name())
                .collect::<Vec<_>>(),
            second_emitted
                .iter()
                .map(|path| path.file_name())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            first_emitted
                .iter()
                .map(|path| fs::read(path).expect("read first schema"))
                .collect::<Vec<_>>(),
            second_emitted
                .iter()
                .map(|path| fs::read(path).expect("read second schema"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_empty_generated_tree_is_not_a_valid_census() {
        let temp = TempDir::new("empty-tree");
        fs::create_dir_all(temp.path()).expect("create empty output directory");
        let error = validate_exact_schema_census(temp.path(), &[])
            .expect_err("empty output must fail closed");
        assert!(error.to_string().contains("nonempty"));
    }
}
