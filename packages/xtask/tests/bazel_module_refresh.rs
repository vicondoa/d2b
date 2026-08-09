#[path = "../src/bazel.rs"]
#[allow(dead_code)]
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
    write_walker: bool,
    startup_error: Option<&'static str>,
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
        if let Some(error) = self.startup_error {
            return Err(error.into());
        }
        if self.write_lock {
            fs::write(root.join("MODULE.bazel.lock"), b"module-lock-v1\n")?;
        }
        if self.write_product {
            fs::create_dir_all(root.join("bazel/cargo"))?;
            fs::write(root.join("bazel/cargo/product.lock"), b"product-lock-v1\n")?;
        }
        if self.write_walker {
            fs::create_dir_all(root.join("bazel/cargo"))?;
            fs::write(root.join("bazel/cargo/walker.lock"), b"walker-lock-v1\n")?;
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
        write_walker: false,
        startup_error: None,
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
        write_walker: false,
        startup_error: None,
    };
    let error = bazel::bazel_module_refresh_with_executor(&root, &mut executor)
        .expect_err("unrelated mutation must refuse");
    assert!(error.to_string().contains("D2B-BZL-UNEXPECTED-MUTATION"));
    assert!(error.to_string().contains("unrelated.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fresh_repin_uses_bzlmod_extension_sync_and_selected_output_isolation() {
    for hub in ["product", "walker"] {
        let root = temp_root(hub);
        let mut executor = Executor {
            calls: Vec::new(),
            write_lock: false,
            unrelated: false,
            write_product: hub == "product",
            write_walker: hub == "walker",
            startup_error: None,
        };
        let output =
            bazel::bazel_repin_with_executor(&root, hub, &mut executor).expect("fresh repin");
        assert_eq!(
            output,
            vec![PathBuf::from(format!("bazel/cargo/{hub}.lock"))]
        );
        assert_eq!(
            executor.calls[0].2,
            vec![
                "mod".to_owned(),
                "deps".to_owned(),
                "--lockfile_mode=off".to_owned(),
            ]
        );
        assert_eq!(
            executor.calls[0].3,
            vec![
                ("CARGO_BAZEL_REPIN".to_owned(), "1".to_owned()),
                ("CARGO_BAZEL_REPIN_ONLY".to_owned(), hub.to_owned()),
            ]
        );
        assert!(!root.join("MODULE.bazel.lock").exists());
        let other = if hub == "product" {
            "bazel/cargo/walker.lock"
        } else {
            "bazel/cargo/product.lock"
        };
        assert!(!root.join(other).exists());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn repin_after_module_refresh_uses_global_lockfile_policy() {
    let root = temp_root("repin-after-module");
    fs::write(root.join("MODULE.bazel.lock"), b"module-lock-v1\n").unwrap();
    let mut executor = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: false,
        write_product: false,
        write_walker: true,
        startup_error: None,
    };
    bazel::bazel_repin_with_executor(&root, "walker", &mut executor)
        .expect("repin after module refresh");
    assert_eq!(
        executor.calls[0].2,
        vec!["mod".to_owned(), "deps".to_owned()]
    );
    assert_eq!(
        executor.calls[0].3,
        vec![
            ("CARGO_BAZEL_REPIN".to_owned(), "1".to_owned()),
            ("CARGO_BAZEL_REPIN_ONLY".to_owned(), "walker".to_owned()),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repin_rejects_an_unselected_hub_lock() {
    let root = temp_root("repin-isolation");
    let mut executor = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: false,
        write_product: true,
        write_walker: false,
        startup_error: None,
    };
    let error = bazel::bazel_repin_with_executor(&root, "walker", &mut executor)
        .expect_err("unselected hub lock must refuse");
    assert!(error.to_string().contains("bazel/cargo/product.lock"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn startup_failures_preserve_actionable_closed_diagnostics() {
    let root = temp_root("startup");
    let mut missing_path = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: false,
        write_product: false,
        write_walker: false,
        startup_error: Some(bazel::bazel_executable_diagnostic()),
    };
    let error = bazel::bazel_module_refresh_with_executor(&root, &mut missing_path)
        .expect_err("missing Bazel executable")
        .to_string();
    assert!(error.contains("status=not-started"));
    assert!(error.contains("D2B-BZL-EXECUTABLE"));
    assert!(error.contains("nix develop"));

    let mut failed_spawn = Executor {
        calls: Vec::new(),
        write_lock: false,
        unrelated: false,
        write_product: false,
        write_walker: false,
        startup_error: Some("spawn failed at /home/operator/private"),
    };
    let error = bazel::bazel_module_refresh_with_executor(&root, &mut failed_spawn)
        .expect_err("failed spawn")
        .to_string();
    assert!(error.contains("spawn failed at <path>"));
    assert!(!error.contains("/home/operator"));
    let _ = fs::remove_dir_all(root);
}
