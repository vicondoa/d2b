use std::collections::BTreeSet;

use d2b_contract_tests::read_repo_file;
use regex::Regex;

#[derive(Clone, Copy)]
struct RunnerCoverage {
    role: &'static str,
    builder_module: &'static str,
    matrix_file: &'static str,
    matrix_marker: &'static str,
}

const RUNNER_COVERAGE: &[RunnerCoverage] = &[
    RunnerCoverage {
        role: "Swtpm",
        builder_module: "swtpm_argv",
        matrix_file: "packages/d2b-contract-tests/tests/runner_shape_contract.rs",
        matrix_marker: "ProcessRole::Swtpm",
    },
    RunnerCoverage {
        role: "Virtiofsd",
        builder_module: "virtiofsd_argv",
        matrix_file: "packages/d2b-contract-tests/tests/runner_shape_contract.rs",
        matrix_marker: "ProcessRole::Virtiofsd",
    },
    RunnerCoverage {
        role: "Video",
        builder_module: "video_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_swtpm_video.rs",
        matrix_marker: "ProcessRole::Video",
    },
    RunnerCoverage {
        role: "Gpu",
        builder_module: "gpu_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_gpu.rs",
        matrix_marker: "ProcessRole::Gpu",
    },
    RunnerCoverage {
        role: "GpuRenderNode",
        builder_module: "gpu_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_swtpm_video.rs",
        matrix_marker: "ProcessRole::GpuRenderNode",
    },
    RunnerCoverage {
        role: "Audio",
        builder_module: "audio_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_audio_usbip.rs",
        matrix_marker: "ProcessRole::Audio",
    },
    RunnerCoverage {
        role: "CloudHypervisorRunner",
        builder_module: "ch_argv",
        matrix_file: "packages/d2b-contract-tests/tests/runner_shape_contract.rs",
        matrix_marker: "ProcessRole::CloudHypervisorRunner",
    },
    RunnerCoverage {
        role: "QemuMediaRunner",
        builder_module: "qemu_media_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_roles.rs",
        matrix_marker: "qemu_media_profile_source_is_fd_backed_and_device_closed",
    },
    RunnerCoverage {
        role: "VsockRelay",
        builder_module: "vsock_relay_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_relay_otel.rs",
        matrix_marker: "ProcessRole::VsockRelay",
    },
    RunnerCoverage {
        role: "OtelHostBridge",
        builder_module: "otel_host_bridge_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_relay_otel.rs",
        matrix_marker: "ProcessRole::OtelHostBridge",
    },
    RunnerCoverage {
        role: "Usbip",
        builder_module: "usbip_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_audio_usbip.rs",
        matrix_marker: "ProcessRole::Usbip",
    },
    RunnerCoverage {
        role: "WaylandProxy",
        builder_module: "wayland_proxy_argv",
        matrix_file: "packages/d2b-contract-tests/tests/minijail_gpu.rs",
        matrix_marker: "ProcessRole::WaylandProxy",
    },
];

const NON_RUNNER_ROLES: &[&str] = &[
    "HostReconcile",
    "StoreVirtiofsPreflight",
    "SwtpmPreStartFlush",
    "GuestSshReadiness",
    "GuestControlHealth",
    "SecurityKeyFrontend",
];

fn process_role_variants() -> Vec<String> {
    let src = read_repo_file("packages/d2b-core/src/processes.rs");
    let re = Regex::new(r"(?s)pub enum ProcessRole\s*\{(?P<body>.*?)\n\}")
        .expect("valid ProcessRole enum regex");
    let body = re
        .captures(&src)
        .and_then(|captures| captures.name("body"))
        .expect("ProcessRole enum body must be parseable")
        .as_str();

    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("///")
                || line.starts_with("//")
                || line.starts_with("#[")
            {
                return None;
            }
            let variant = line.split_once(',').map_or(line, |(variant, _)| variant);
            let variant = variant
                .split_once('{')
                .map_or(variant, |(variant, _)| variant)
                .trim();
            (!variant.is_empty()).then(|| variant.to_owned())
        })
        .collect()
}

fn assert_unique(items: impl Iterator<Item = &'static str>, label: &str) {
    let mut seen = BTreeSet::new();
    let mut duplicates = Vec::new();
    for item in items {
        if !seen.insert(item) {
            duplicates.push(item);
        }
    }
    assert!(
        duplicates.is_empty(),
        "{label} has duplicates: {duplicates:?}"
    );
}

#[test]
fn every_process_role_is_classified_for_runner_coverage_policy() {
    assert_unique(
        RUNNER_COVERAGE.iter().map(|coverage| coverage.role),
        "runner coverage table",
    );
    assert_unique(NON_RUNNER_ROLES.iter().copied(), "non-runner role table");

    let declared: BTreeSet<_> = process_role_variants().into_iter().collect();
    let covered: BTreeSet<_> = RUNNER_COVERAGE
        .iter()
        .map(|coverage| coverage.role.to_owned())
        .chain(NON_RUNNER_ROLES.iter().map(|role| (*role).to_owned()))
        .collect();

    let missing: Vec<_> = declared.difference(&covered).cloned().collect();
    let stale: Vec<_> = covered.difference(&declared).cloned().collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "ProcessRole coverage policy drifted; missing rows for {missing:?}, stale rows {stale:?}. \
         New runner roles must add a Rust argv builder and runner matrix/contract coverage."
    );
}

#[test]
fn runner_process_roles_have_builder_and_matrix_contract_coverage() {
    let host_lib = read_repo_file("packages/d2b-host/src/lib.rs");
    let regenerator = read_repo_file("packages/d2b-host/src/runner_argv_regenerator.rs");
    let generator_fn =
        Regex::new(r"pub fn generate_[a-z0-9_]*argv").expect("valid generator function regex");

    for coverage in RUNNER_COVERAGE {
        assert!(
            host_lib.contains(&format!("pub mod {};", coverage.builder_module)),
            "ProcessRole::{} must expose d2b-host::{} as its Rust argv builder",
            coverage.role,
            coverage.builder_module
        );

        let builder_src = read_repo_file(&format!(
            "packages/d2b-host/src/{}.rs",
            coverage.builder_module
        ));
        assert!(
            generator_fn.is_match(&builder_src),
            "ProcessRole::{} builder module {} must expose a generate_*argv function",
            coverage.role,
            coverage.builder_module
        );

        assert!(
            regenerator.contains(&format!("ProcessRole::{}", coverage.role)),
            "ProcessRole::{} must be classified by runner_argv_regenerator",
            coverage.role
        );

        let matrix_src = read_repo_file(coverage.matrix_file);
        let marker = Regex::new(coverage.matrix_marker).unwrap_or_else(|err| {
            panic!("invalid marker for ProcessRole::{}: {err}", coverage.role)
        });
        assert!(
            marker.is_match(&matrix_src),
            "ProcessRole::{} must have runner matrix/contract coverage in {}",
            coverage.role,
            coverage.matrix_file
        );
    }
}
