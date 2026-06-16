use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

use nixling_contract_tests::load_full_bundle_resolver_from_env;
use nixling_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessNode, VmProcessDag},
};

fn full_resolver_or_skip(test: &str) -> Option<BundleResolver> {
    match load_full_bundle_resolver_from_env() {
        Some(resolver) => Some(resolver),
        None => {
            eprintln!(
                "SKIP {test}: NL_FIXTURES_FULL unset (video binary-contract fixture unavailable)"
            );
            None
        }
    }
}

fn graphics_video_dag<'a>(resolver: &'a BundleResolver, test: &str) -> &'a VmProcessDag {
    let matches: Vec<&VmProcessDag> = resolver
        .processes
        .vms
        .iter()
        .filter(|dag| {
            dag.nodes.iter().any(|node| node.id.0.as_str() == "video")
                && dag
                    .nodes
                    .iter()
                    .any(|node| node.id.0.as_str() == "cloud-hypervisor")
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "{test}: fixture-smoke-full must render exactly one graphics/video VM with video + cloud-hypervisor nodes; saw {:?}",
        matches.iter().map(|dag| dag.vm.as_str()).collect::<Vec<_>>()
    );
    matches[0]
}

fn node_by_id<'a>(dag: &'a VmProcessDag, id: &str, test: &str) -> &'a ProcessNode {
    let matches: Vec<&ProcessNode> = dag
        .nodes
        .iter()
        .filter(|node| node.id.0.as_str() == id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "{test}: VM {} must render exactly one node with id {id:?}",
        dag.vm
    );
    matches[0]
}

fn executable_binary_path<'a>(node: &'a ProcessNode, label: &str, test: &str) -> &'a str {
    let path = node
        .binary_path
        .as_deref()
        .unwrap_or_else(|| panic!("{test}: {label} node is missing binaryPath"));
    let metadata = fs::metadata(path)
        .unwrap_or_else(|err| panic!("{test}: {label} binary missing at {path}: {err}"));
    assert!(
        metadata.is_file(),
        "{test}: {label} binaryPath is not a regular file: {path}"
    );
    assert_ne!(
        metadata.permissions().mode() & 0o111,
        0,
        "{test}: {label} binaryPath is not executable: {path}"
    );
    path
}

fn run_help(path: &str, args: &[&str], label: &str, test: &str) -> Output {
    Command::new(Path::new(path))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("{test}: failed to run {label} help at {path}: {err}"))
}

fn combined_output(output: &Output) -> String {
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    combined
}

#[test]
fn patched_video_binaries_expose_required_command_surface() {
    let test = "patched_video_binaries_expose_required_command_surface";
    let Some(resolver) = full_resolver_or_skip(test) else {
        return;
    };
    let dag = graphics_video_dag(&resolver, test);

    let video_bin = executable_binary_path(node_by_id(dag, "video", test), "video crosvm", test);
    let ch_bin = executable_binary_path(
        node_by_id(dag, "cloud-hypervisor", test),
        "cloud-hypervisor",
        test,
    );

    let video_help = run_help(
        video_bin,
        &["device", "video-decoder", "--help"],
        "crosvm device video-decoder",
        test,
    );
    let video_help_text = combined_output(&video_help);
    assert!(
        video_help.status.success() && video_help_text.contains("--backend"),
        "{test}: patched crosvm must expose `device video-decoder --backend`; status {:?}; output:\n{}",
        video_help.status.code(),
        video_help_text
    );

    let ch_help = run_help(ch_bin, &["--help"], "cloud-hypervisor", test);
    let ch_help_text = combined_output(&ch_help);
    assert!(
        ch_help.status.success() && ch_help_text.contains("--vhost-user-media"),
        "{test}: patched Cloud Hypervisor must expose `--vhost-user-media`; status {:?}; output:\n{}",
        ch_help.status.code(),
        ch_help_text
    );
}
