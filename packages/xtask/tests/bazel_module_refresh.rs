#[path = "../src/bazel.rs"]
mod bazel;
#[path = "../src/hermeticity.rs"]
mod hermeticity;

use std::{
    fs,
    path::{Path, PathBuf},
};

type Call = (PathBuf, Vec<String>, Vec<String>, Vec<(String, String)>);

struct Executor {
    calls: Vec<Call>,
    write_lock: bool,
    unrelated: bool,
    write_product: bool,
}

impl bazel::BazelExecutor for Executor {
    fn run(
        &mut self,
        root: &Path,
        startup_args: &[String],
        command_args: &[String],
        environment: &[(&str, &str)],
    ) -> Result<std::process::ExitStatus, Box<dyn std::error::Error>> {
        self.calls.push((
            root.to_path_buf(),
            startup_args.to_vec(),
            command_args.to_vec(),
            environment
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        ));
        if self.write_lock {
            fs::write(root.join("MODULE.bazel.lock"), b"module-lock-v1\n")?;
        }
        if self.write_product {
            fs::create_dir_all(root.join("bazel/cargo"))?;
            fs::write(root.join("bazel/cargo/product.lock"), b"product-lock-v1\n")?;
        }
        if self.unrelated {
            fs::write(root.join("unrelated.txt"), b"unexpected\n")?;
        }
        Ok(std::process::Command::new("true").status()?)
    }
}

fn temp_root(label: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(".scratch")
        .join(format!("module-refresh-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn refresh_is_no_argument_absolute_and_lock_only() {
    let root = temp_root("positive");
    let mut executor = Executor {
        calls: Vec::new(),
        write_lock: true,
        unrelated: false,
        write_product: false,
    };
    let first =
        bazel::bazel_module_refresh_with_executor(&root, &mut executor).expect("first refresh");
    assert_eq!(first, vec![PathBuf::from("MODULE.bazel.lock")]);
    assert_eq!(executor.calls.len(), 1);
    assert_eq!(
        executor.calls[0].2,
        vec![
            "mod".to_owned(),
            "deps".to_owned(),
            "--lockfile_mode=update".to_owned()
        ]
    );
    assert_eq!(
        executor.calls[0].1,
        vec![
            format!(
                "--output_user_root={}/.scratch/bazel/output-user-root",
                root.display()
            ),
            format!(
                "--output_base={}/.scratch/bazel/output-base",
                root.display()
            ),
        ]
    );
    assert!(executor.calls[0].3.is_empty());

    executor.write_lock = false;
    let second =
        bazel::bazel_module_refresh_with_executor(&root, &mut executor).expect("second refresh");
    assert!(second.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_refuses_unrelated_mutation() {
    let root = temp_root("unrelated");
    let mut executor = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: true,
        write_product: false,
    };
    let error = bazel::bazel_module_refresh_with_executor(&root, &mut executor)
        .expect_err("unrelated mutation must refuse");
    assert!(error.to_string().contains("D2B-BZL-UNEXPECTED-MUTATION"));
    assert!(error.to_string().contains("unrelated.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fresh_repin_uses_command_local_off_and_refuses_it_after_module_refresh() {
    let root = temp_root("repin");
    let mut executor = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: false,
        write_product: true,
    };
    let output =
        bazel::bazel_repin_with_executor(&root, "product", &mut executor).expect("fresh repin");
    assert_eq!(output, vec![PathBuf::from("bazel/cargo/product.lock")]);
    assert_eq!(
        executor.calls[0].2,
        vec![
            "run".to_owned(),
            "--lockfile_mode=off".to_owned(),
            format!(
                "--symlink_prefix={}/.scratch/bazel/symlinks/",
                root.display()
            ),
            "@rules_rust//crate_universe:cargo_bazel".to_owned(),
            "--".to_owned(),
            "generate".to_owned(),
        ]
    );
    assert_eq!(
        executor.calls[0].3,
        vec![
            ("CARGO_BAZEL_REPIN".to_owned(), "1".to_owned()),
            ("CARGO_BAZEL_REPIN_ONLY".to_owned(), "product".to_owned()),
        ]
    );

    fs::write(root.join("MODULE.bazel.lock"), b"module-lock-v1\n").unwrap();
    executor.calls.clear();
    executor.write_product = false;
    bazel::bazel_repin_with_executor(&root, "walker", &mut executor)
        .expect("repin after module refresh");
    assert!(
        !executor.calls[0]
            .2
            .iter()
            .any(|argument| argument == "--lockfile_mode=off")
    );
    let _ = fs::remove_dir_all(root);
}
