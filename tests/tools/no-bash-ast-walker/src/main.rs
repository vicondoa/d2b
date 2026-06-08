use std::path::Path;
use walkdir::WalkDir;

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

    let mut violations: Vec<String> = Vec::new();
    let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !matches!(name.as_ref(), "target" | "tests" | "fixtures" | ".git")
    });

    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let src = match std::fs::read_to_string(path) {
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
        eprintln!("no-bash-ast-walker: FAIL — {} violation(s):", violations.len());
        for v in &violations {
            eprintln!("  {v}");
        }
        std::process::exit(1);
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
            if is_command_new && call.args.len() == 1 {
                if let syn::Expr::Lit(lit) = &call.args[0] {
                    if let syn::Lit::Str(s) = &lit.lit {
                        let val = s.value();
                        if BASH_LITERALS.iter().any(|b| val == *b) {
                            self.findings
                                .push(format!("{}: Command::new({val:?})", self.path.display()));
                        }
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}
