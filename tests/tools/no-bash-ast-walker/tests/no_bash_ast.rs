use std::path::PathBuf;

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        let root = PathBuf::from(root);
        if root.join("Cargo.toml").is_file() {
            return root;
        }
    }
    if let (Some(srcdir), Some(workspace)) = (
        std::env::var_os("TEST_SRCDIR"),
        std::env::var_os("TEST_WORKSPACE"),
    ) {
        let root = PathBuf::from(srcdir).join(workspace);
        if root.join("Cargo.toml").is_file() {
            return root;
        }
    }
    let mut root = std::env::current_dir().expect("resolve current directory");
    loop {
        if root.join("Cargo.toml").is_file() {
            return root;
        }
        assert!(
            root.pop(),
            "no-bash AST test must have a repository ancestor"
        );
    }
}

#[test]
fn packages_have_no_bash_command_literals() {
    let root = repository_root();
    let violations = no_bash_ast_walker::violations(&root.join("packages"));
    assert!(
        violations.is_empty(),
        "no-bash AST policy found forbidden command literals:\n{}",
        violations.join("\n")
    );
}
