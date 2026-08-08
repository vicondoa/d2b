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
const AUTHORITATIVE_SCHEMA_ROOT: &str = "docs/reference/schemas/v2";
const AUTHORITATIVE_SCHEMA_DOCS: &[&str] = &[
    "allocator.md",
    "bundle.md",
    "closures.md",
    "d2b-realm-core.md",
    "guest-control.md",
    "host.md",
    "manifest_v04.md",
    "minijail-profile.md",
    "privileges.md",
    "processes.md",
    "realm-controllers.md",
    "realm-identity.md",
    "realm-workloads-launcher-v2.md",
    "storage.md",
    "sync.md",
    "unsafe-local-helper-wire.md",
    "unsafe-local-workloads.md",
    "wire-protocol.md",
];
const SCHEMA_DRIFT_REMEDIATION: &str = "\
D2B-SCHEMA-DRIFT: committed schema output is not an exact, nonempty census.
From the repository root, run: nix develop
Then run from packages/: cargo xtask gen-schemas
Review and commit the repository-relative schema changes, then rerun the failed command.";

pub(crate) fn gen_schemas_with_args(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let explicit_output = parse_gen_schemas_args(args)?;
    let repo_root = super::repo_root()?;
    let output_dir = match explicit_output.as_ref() {
        Some(path) => {
            let path = resolve_output_dir(path.to_owned())?;
            let scratch = repo_root.join(".scratch");
            if path.strip_prefix(&scratch).is_err() {
                return Err(
                    "D2B-SCHEMA-OUTPUT: explicit schema output must be under .scratch/.".into(),
                );
            }
            path
        }
        None => repo_root.join(AUTHORITATIVE_SCHEMA_ROOT),
    };

    ensure_output_directory(&output_dir)?;
    let definitions = schema_definitions();
    let emitted = write_schema_files(&output_dir, &definitions)?;
    if explicit_output.is_some() {
        validate_exact_schema_census(&output_dir, &emitted)?;
    } else {
        reconcile_authoritative_schema_json(&output_dir, &emitted)?;
        validate_authoritative_schema_census(&output_dir, &emitted)?;
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
        return Err("D2B-SCHEMA-OUTPUT: schema output directory is not a directory.".into());
    }
    fs::create_dir_all(output_dir)
        .map_err(|_| "D2B-SCHEMA-OUTPUT: could not create the schema output directory.")?;
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
        return Err("D2B-SCHEMA-DRIFT: schema census must be nonempty.".into());
    }
    if !output_dir.is_dir() {
        return Err("D2B-SCHEMA-DRIFT: schema output directory is not a directory.".into());
    }

    let mut expected = BTreeSet::<OsString>::new();
    for path in census {
        if path.parent() != Some(output_dir) {
            return Err(
                "D2B-SCHEMA-DRIFT: schema census path is outside its output directory.".into(),
            );
        }
        let name = path
            .file_name()
            .ok_or("D2B-SCHEMA-DRIFT: schema census path has no file name.")?
            .to_os_string();
        if !expected.insert(name) {
            return Err("D2B-SCHEMA-DRIFT: schema census contains a duplicate path.".into());
        }

        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is missing.")?;
        if !metadata.file_type().is_file() {
            return Err("D2B-SCHEMA-DRIFT: schema output is not a regular file.".into());
        }
        let contents =
            fs::read(path).map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?;
        if contents.is_empty() {
            return Err("D2B-SCHEMA-DRIFT: schema output is empty.".into());
        }
        serde_json::from_slice::<serde_json::Value>(&contents)
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is not valid JSON.")?;
    }

    let mut actual = BTreeSet::<OsString>::new();
    for entry in
        fs::read_dir(output_dir).map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?
    {
        let entry = entry.map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?;
        let name = entry.file_name();
        if !entry
            .file_type()
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?
            .is_file()
        {
            return Err(
                "D2B-SCHEMA-DRIFT: schema output directory contains a non-file entry.".into(),
            );
        }
        actual.insert(name);
    }
    if actual != expected {
        return Err(SCHEMA_DRIFT_REMEDIATION.into());
    }
    Ok(())
}

fn reconcile_authoritative_schema_json(
    output_dir: &Path,
    census: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    let mut expected = census
        .iter()
        .filter_map(|path| path.file_name().map(OsString::from))
        .collect::<BTreeSet<_>>();
    expected.extend(AUTHORITATIVE_SCHEMA_DOCS.iter().map(OsString::from));
    if expected.is_empty() {
        return Err(SCHEMA_DRIFT_REMEDIATION.into());
    }
    for entry in fs::read_dir(output_dir).map_err(|_| SCHEMA_DRIFT_REMEDIATION)? {
        let entry = entry.map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        let name = entry.file_name();
        let file_type = entry.file_type().map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        if !file_type.is_file() {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        if !expected.contains(&name) {
            fs::remove_file(entry.path()).map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        }
    }
    Ok(())
}

fn validate_authoritative_schema_census(
    output_dir: &Path,
    census: &[PathBuf],
) -> Result<(), Box<dyn Error>> {
    if census.is_empty() {
        return Err(SCHEMA_DRIFT_REMEDIATION.into());
    }
    let metadata = fs::symlink_metadata(output_dir).map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
    if !metadata.file_type().is_dir() {
        return Err(SCHEMA_DRIFT_REMEDIATION.into());
    }

    let mut expected = census
        .iter()
        .filter_map(|path| path.file_name().map(OsString::from))
        .collect::<BTreeSet<_>>();
    expected.extend(AUTHORITATIVE_SCHEMA_DOCS.iter().map(OsString::from));
    let mut actual = BTreeSet::new();
    for path in census {
        let name = path.file_name().ok_or(SCHEMA_DRIFT_REMEDIATION)?;
        let metadata = fs::symlink_metadata(path).map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        if !metadata.file_type().is_file() {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        let contents = fs::read(path).map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        if contents.is_empty() || serde_json::from_slice::<serde_json::Value>(&contents).is_err() {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        actual.insert(name.to_os_string());
    }
    for doc in AUTHORITATIVE_SCHEMA_DOCS {
        let path = output_dir.join(doc);
        let metadata = fs::symlink_metadata(&path).map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        if !metadata.file_type().is_file() {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        if fs::read(&path)
            .map_err(|_| SCHEMA_DRIFT_REMEDIATION)?
            .is_empty()
        {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        actual.insert(OsString::from(doc));
    }
    for entry in fs::read_dir(output_dir).map_err(|_| SCHEMA_DRIFT_REMEDIATION)? {
        let entry = entry.map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        let name = entry.file_name();
        let file_type = entry.file_type().map_err(|_| SCHEMA_DRIFT_REMEDIATION)?;
        if !file_type.is_file() {
            return Err(SCHEMA_DRIFT_REMEDIATION.into());
        }
        actual.insert(name);
    }
    if actual != expected {
        return Err(SCHEMA_DRIFT_REMEDIATION.into());
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
        return Err("D2B-SCHEMA-DRIFT: independent schema census must be nonempty.".into());
    }
    if first_files.keys().collect::<BTreeSet<_>>() != second_files.keys().collect::<BTreeSet<_>>() {
        return Err("D2B-SCHEMA-DRIFT: independent schema censuses differ.".into());
    }
    for name in first_files.keys() {
        let first_contents = fs::read(first.join(name))
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?;
        let second_contents = fs::read(second.join(name))
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?;
        serde_json::from_slice::<serde_json::Value>(&first_contents)
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is not valid JSON.")?;
        serde_json::from_slice::<serde_json::Value>(&second_contents)
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is not valid JSON.")?;
        if first_contents != second_contents {
            return Err("D2B-SCHEMA-DRIFT: independent schema output differs.".into());
        }
    }
    Ok(())
}

fn schema_files(directory: &Path) -> Result<BTreeMap<OsString, PathBuf>, Box<dyn Error>> {
    if !directory.is_dir() {
        return Err("D2B-SCHEMA-DRIFT: schema output directory is not a directory.".into());
    }
    let mut files = BTreeMap::new();
    for entry in
        fs::read_dir(directory).map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?
    {
        let entry = entry.map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?;
        if !entry
            .file_type()
            .map_err(|_| "D2B-SCHEMA-DRIFT: schema output is unreadable.")?
            .is_file()
        {
            return Err("D2B-SCHEMA-DRIFT: schema output contains a non-file entry.".into());
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
        AUTHORITATIVE_SCHEMA_DOCS, gen_schemas_with_args, parse_gen_schemas_args,
        validate_authoritative_schema_census, validate_exact_schema_census, write_schema_files,
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

    #[test]
    fn authoritative_schema_census_rejects_missing_extra_and_absent_roots() {
        let temp = TempDir::new("authoritative-census");
        let output = temp.path().join("schemas");
        fs::create_dir_all(&output).expect("create authoritative output directory");
        let definitions = vec![(
            "first.json",
            schemars::schema_for!(d2b_core::allocator_config::AllocatorJson),
        )];
        let emitted =
            write_schema_files(&output, &definitions).expect("write authoritative schema");
        for name in AUTHORITATIVE_SCHEMA_DOCS {
            fs::write(output.join(name), b"documented schema\n").expect("write schema sidecar");
        }
        validate_authoritative_schema_census(&output, &emitted)
            .expect("authoritative census is exact");

        fs::remove_file(&emitted[0]).expect("remove expected schema");
        assert!(
            validate_authoritative_schema_census(&output, &emitted).is_err(),
            "missing schema must fail closed"
        );

        write_schema_files(&output, &definitions).expect("restore expected schema");
        fs::write(output.join("stale.json"), b"{}\n").expect("write stale schema");
        assert!(
            validate_authoritative_schema_census(&output, &emitted).is_err(),
            "extra schema must fail closed"
        );

        let absent = output.join("absent");
        assert!(
            validate_authoritative_schema_census(&absent, &emitted).is_err(),
            "absent schema root must fail closed"
        );
    }
}
