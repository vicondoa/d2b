use std::path::Path;

const BASH_LITERALS: &[&str] = &[
    "bash",
    "sh",
    "/bin/bash",
    "/bin/sh",
    "/usr/bin/bash",
    "/usr/bin/sh",
    "/usr/bin/env bash",
    "/usr/bin/env sh",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .get(1)
        .map(Path::new)
        .unwrap_or_else(|| Path::new("packages"));

    let mut paths = Vec::new();
    collect_rust_files(root, &mut paths);
    paths.sort();

    let mut violations: Vec<String> = Vec::new();
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let parsed = match syn::parse_file(&src) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut visitor = BashLiteralVisitor {
            path: path.to_path_buf(),
            findings: Vec::new(),
        };
        syn::visit::Visit::visit_file(&mut visitor, &parsed);
        violations.extend(visitor.findings);
    }

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

fn collect_rust_files(root: &Path, files: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), "target" | "tests" | "fixtures" | ".git") {
            continue;
        }
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

struct BashLiteralVisitor {
    path: std::path::PathBuf,
    findings: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for BashLiteralVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path_expr) = &*call.func {
            let segs: Vec<String> = path_expr
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let is_command_new = segs.windows(2).any(|w| w == ["Command", "new"]);
            if is_command_new
                && call.args.len() == 1
                && let syn::Expr::Lit(lit) = &call.args[0]
                && let syn::Lit::Str(s) = &lit.lit
            {
                let val = s.value();
                if BASH_LITERALS.iter().any(|b| val == *b) {
                    self.findings
                        .push(format!("{}: Command::new({val:?})", self.path.display()));
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}
