use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use syn::visit::Visit;
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

const LOCAL_RUNTIME_SENSITIVE_IDENTIFIERS: &[&str] = &[
    "ChArgvInput",
    "QemuMediaArgvInput",
    "argv",
    "credential",
    "endpoint",
    "extra_args",
    "host_path",
    "serde_json",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PolicyTarget {
    General,
    LocalRuntime,
    DaemonProviderEffects,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .get(1)
        .map(Path::new)
        .unwrap_or_else(|| Path::new("packages"));

    let mut violations = Vec::new();
    let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        !matches!(name.as_ref(), "target" | "tests" | "fixtures" | ".git")
    });

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                violations.push(format!("walk failed: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                violations.push(format!("{}: read failed: {error}", path.display()));
                continue;
            }
        };
        let target = policy_target(path);
        match source_policy_violations(&source, path, target) {
            Ok(mut findings) => violations.append(&mut findings),
            Err(error) => violations.push(format!("{}: parse failed: {error}", path.display())),
        }
    }

    violations.sort();
    violations.dedup();
    if violations.is_empty() {
        println!(
            "no-bash-ast-walker: PASS (bash and exact source policies in {})",
            root.display()
        );
        std::process::exit(0);
    }

    eprintln!(
        "no-bash-ast-walker: FAIL — {} violation(s):",
        violations.len()
    );
    for violation in &violations {
        eprintln!("  {violation}");
    }
    std::process::exit(1);
}

fn policy_target(path: &Path) -> PolicyTarget {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    if components
        .windows(2)
        .any(|pair| pair == ["d2b-provider-runtime-local", "src"])
    {
        PolicyTarget::LocalRuntime
    } else if components
        .windows(3)
        .any(|parts| parts == ["d2bd", "src", "provider_effects.rs"])
    {
        PolicyTarget::DaemonProviderEffects
    } else {
        PolicyTarget::General
    }
}

fn source_policy_violations(
    source: &str,
    path: &Path,
    target: PolicyTarget,
) -> syn::Result<Vec<String>> {
    let parsed = syn::parse_file(source)?;
    let mut aliases = AliasCollector::default();
    aliases.visit_file(&parsed);
    let mut visitor = SourcePolicyVisitor {
        path: path.to_path_buf(),
        target,
        aliases: aliases.aliases,
        findings: Vec::new(),
    };
    visitor.visit_file(&parsed);
    visitor.findings.sort();
    visitor.findings.dedup();
    Ok(visitor.findings)
}

#[derive(Default)]
struct AliasCollector {
    aliases: BTreeMap<String, Vec<String>>,
}

impl AliasCollector {
    fn collect_use_tree(&mut self, prefix: &[String], tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(path) => {
                let mut next = prefix.to_vec();
                next.push(path.ident.to_string());
                self.collect_use_tree(&next, &path.tree);
            }
            syn::UseTree::Name(name) => {
                if name.ident == "self" {
                    if let Some(local) = prefix.last() {
                        self.aliases.insert(local.clone(), prefix.to_vec());
                    }
                } else {
                    let mut target = prefix.to_vec();
                    target.push(name.ident.to_string());
                    self.aliases.insert(name.ident.to_string(), target);
                }
            }
            syn::UseTree::Rename(rename) => {
                let mut target = prefix.to_vec();
                if rename.ident != "self" {
                    target.push(rename.ident.to_string());
                }
                self.aliases.insert(rename.rename.to_string(), target);
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.collect_use_tree(prefix, item);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for AliasCollector {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.collect_use_tree(&[], &item.tree);
        syn::visit::visit_item_use(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        if let syn::Type::Path(target) = &*item.ty {
            self.aliases.insert(
                item.ident.to_string(),
                path_segments(&target.path).collect(),
            );
        }
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        let local = item
            .rename
            .as_ref()
            .map_or_else(|| item.ident.to_string(), |(_, rename)| rename.to_string());
        self.aliases.insert(local, vec![item.ident.to_string()]);
        syn::visit::visit_item_extern_crate(self, item);
    }
}

struct SourcePolicyVisitor {
    path: PathBuf,
    target: PolicyTarget,
    aliases: BTreeMap<String, Vec<String>>,
    findings: Vec<String>,
}

impl SourcePolicyVisitor {
    fn finding(&mut self, policy: &str, construct: &str) {
        self.findings
            .push(format!("{}: {policy}: {construct}", self.path.display()));
    }

    fn resolved_segments(&self, segments: &[String]) -> Vec<String> {
        let mut resolved = segments.to_vec();
        for _ in 0..=self.aliases.len() {
            let Some(replacement) = resolved.first().and_then(|first| self.aliases.get(first))
            else {
                break;
            };
            let mut next = replacement.clone();
            next.extend(resolved.iter().skip(1).cloned());
            if next == resolved {
                break;
            }
            resolved = next;
        }
        resolved
    }

    fn check_path_segments(&mut self, segments: &[String]) {
        let resolved = self.resolved_segments(segments);
        if self.target != PolicyTarget::General
            && (has_adjacent(&resolved, "std", "process")
                || has_adjacent(&resolved, "Command", "new"))
        {
            self.finding("process construction is forbidden", &resolved.join("::"));
        }
        if self.target == PolicyTarget::DaemonProviderEffects
            && resolved.iter().any(|segment| segment == "d2b_priv_broker")
        {
            self.finding(
                "direct privileged broker access is forbidden",
                &resolved.join("::"),
            );
        }
        if self.target == PolicyTarget::LocalRuntime {
            for segment in &resolved {
                self.check_local_runtime_identifier(segment);
            }
        }
    }

    fn check_local_runtime_identifier(&mut self, identifier: &str) {
        if LOCAL_RUNTIME_SENSITIVE_IDENTIFIERS.contains(&identifier) {
            self.finding("local runtime operation input is not opaque", identifier);
        }
    }

    fn visit_use_paths(&mut self, prefix: &[String], tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(path) => {
                let mut next = prefix.to_vec();
                next.push(path.ident.to_string());
                self.visit_use_paths(&next, &path.tree);
            }
            syn::UseTree::Name(name) => {
                let mut path = prefix.to_vec();
                if name.ident != "self" {
                    path.push(name.ident.to_string());
                }
                self.check_path_segments(&path);
            }
            syn::UseTree::Rename(rename) => {
                let mut path = prefix.to_vec();
                if rename.ident != "self" {
                    path.push(rename.ident.to_string());
                }
                self.check_path_segments(&path);
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.visit_use_paths(prefix, item);
                }
            }
            syn::UseTree::Glob(_) => self.check_path_segments(prefix),
        }
    }
}

impl<'ast> Visit<'ast> for SourcePolicyVisitor {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path_expression) = &*call.func {
            let segments = path_segments(&path_expression.path).collect::<Vec<_>>();
            let resolved = self.resolved_segments(&segments);
            if has_adjacent(&resolved, "Command", "new")
                && call.args.len() == 1
                && let syn::Expr::Lit(literal) = &call.args[0]
                && let syn::Lit::Str(value) = &literal.lit
                && BASH_LITERALS.contains(&value.value().as_str())
            {
                self.finding(
                    "shell execution is forbidden",
                    &format!("{}({:?})", resolved.join("::"), value.value()),
                );
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path_segments(path).collect::<Vec<_>>();
        self.check_path_segments(&segments);
        syn::visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.visit_use_paths(&[], &item.tree);
        syn::visit::visit_item_use(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.check_path_segments(&[item.ident.to_string()]);
        syn::visit::visit_item_extern_crate(self, item);
    }

    fn visit_ident(&mut self, identifier: &'ast syn::Ident) {
        if self.target == PolicyTarget::LocalRuntime {
            self.check_local_runtime_identifier(&identifier.to_string());
        }
        syn::visit::visit_ident(self, identifier);
    }

    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        if self.target == PolicyTarget::LocalRuntime {
            self.check_local_runtime_identifier(&literal.value());
        }
        syn::visit::visit_lit_str(self, literal);
    }
}

fn path_segments(path: &syn::Path) -> impl Iterator<Item = String> + '_ {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
}

fn has_adjacent(segments: &[String], left: &str, right: &str) -> bool {
    segments
        .windows(2)
        .any(|pair| pair[0] == left && pair[1] == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(source: &str, target: PolicyTarget) -> Vec<String> {
        source_policy_violations(source, Path::new("fixture.rs"), target)
            .expect("fixture must parse")
    }

    #[test]
    fn qualified_and_whitespace_split_process_calls_are_rejected() {
        let source = r#"
            fn bypass() {
                let _ = ::std::process::Command
                    :: new("not-a-shell");
            }
        "#;
        let findings = findings(source, PolicyTarget::DaemonProviderEffects);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("process construction is forbidden"))
        );
    }

    #[test]
    fn command_and_broker_import_aliases_are_rejected() {
        let source = r#"
            use ::std::process::Command as ProcessBuilder;
            use ::d2b_priv_broker as broker;

            fn bypass() {
                let _ = ProcessBuilder :: new("not-a-shell");
                broker::dispatch();
            }
        "#;
        let findings = findings(source, PolicyTarget::DaemonProviderEffects);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("process construction is forbidden"))
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("direct privileged broker access is forbidden"))
        );
    }

    #[test]
    fn extern_crate_broker_alias_is_rejected() {
        let source = r#"
            extern crate d2b_priv_broker as broker;
        "#;
        let findings = findings(source, PolicyTarget::DaemonProviderEffects);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("direct privileged broker access is forbidden"))
        );
    }

    #[test]
    fn module_and_type_alias_chains_are_rejected() {
        let source = r#"
            use std as standard;
            type ProcessBuilder = standard::process::Command;

            fn bypass() {
                let _ = ProcessBuilder::new("not-a-shell");
            }
        "#;
        let findings = findings(source, PolicyTarget::DaemonProviderEffects);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("process construction is forbidden"))
        );
    }

    #[test]
    fn local_runtime_aliases_and_sensitive_variants_are_rejected() {
        let source = r#"
            use serde_json as wire_format;
            use contract::ProviderOperationInput::ChArgvInput as Input;

            struct Request {
                argv: Vec<String>,
                host_path: String,
                endpoint: String,
                credential: String,
                extra_args: Vec<String>,
            }

            fn decode(input: Input) {
                let _ = wire_format::from_str::<Request>("{}");
                let _ = input;
            }
        "#;
        let findings = findings(source, PolicyTarget::LocalRuntime);
        for forbidden in [
            "serde_json",
            "ChArgvInput",
            "argv",
            "host_path",
            "endpoint",
            "credential",
            "extra_args",
        ] {
            assert!(
                findings.iter().any(|finding| finding.ends_with(forbidden)),
                "missing finding for {forbidden}: {findings:?}"
            );
        }
    }

    #[test]
    fn comments_and_longer_identifiers_are_not_policy_constructs() {
        let source = r#"
            // Command::new, d2b_priv_broker, argv, host_path, endpoint, credential
            struct EndpointRole;
            fn command_new_policy_name() {
                let credential_cache = "d2b_priv_broker is only prose";
                let _ = (EndpointRole, credential_cache);
            }
        "#;
        assert!(findings(source, PolicyTarget::DaemonProviderEffects).is_empty());
        assert!(findings(source, PolicyTarget::LocalRuntime).is_empty());
    }
}
