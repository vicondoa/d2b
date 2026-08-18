use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .get(1)
        .map(Path::new)
        .unwrap_or_else(|| Path::new("packages"));

    let violations = no_bash_ast_walker::violations(root);

    if violations.is_empty() {
        println!(
            "no-bash-ast-walker: PASS (zero Command::new bash-literal sites in {})",
            root.display()
        );
        std::process::exit(0);
    } else {
        eprintln!(
            "no-bash-ast-walker: FAIL - {} violation(s):",
            violations.len()
        );
        for v in &violations {
            eprintln!("  {v}");
        }
        std::process::exit(1);
    }
}
