use std::sync::LazyLock;

use nixling_contract_tests::load_full_bundle_resolver_from_env;
use nixling_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessNode, ProcessRole},
};
use regex::Regex;

static NIX_STORE_HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/nix/store/[a-z0-9]{32}-").expect("valid store regex"));
static RUN_USER_UID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/run/user/[0-9]+").expect("valid run/user regex"));

fn full_resolver_or_skip(test: &str) -> Option<BundleResolver> {
    match load_full_bundle_resolver_from_env() {
        Some(resolver) => Some(resolver),
        None => {
            eprintln!("SKIP {test}: NL_FIXTURES_FULL unset (runner-shape fixture unavailable)");
            None
        }
    }
}

fn first_node_with_role(
    resolver: &BundleResolver,
    role: ProcessRole,
) -> Option<(&str, &ProcessNode)> {
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role == role {
                return Some((dag.vm.as_str(), node));
            }
        }
    }
    None
}

fn normalize_arg(arg: &str) -> String {
    let normalized = NIX_STORE_HASH.replace_all(arg, "/nix/store/<HASH>-");
    RUN_USER_UID
        .replace_all(&normalized, "/run/user/<UID>")
        .into_owned()
}

fn normalized_argv(node: &ProcessNode) -> Vec<String> {
    node.argv.iter().map(|arg| normalize_arg(arg)).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn assert_role_shape(
    test_name: &str,
    role: ProcessRole,
    expected_vm: &str,
    expected_argv: &[&str],
    expected_device_binds: &[&str],
) {
    let Some(resolver) = full_resolver_or_skip(test_name) else {
        return;
    };
    let (vm, node) = first_node_with_role(&resolver, role)
        .unwrap_or_else(|| panic!("fixture-smoke-full has no {test_name} node"));

    assert_eq!(
        vm, expected_vm,
        "{test_name} first matching VM drifted from the runner-shape snapshot source"
    );
    assert_eq!(
        normalized_argv(node),
        strings(expected_argv),
        "{test_name} normalized argv drifted from the runner-shape snapshot"
    );
    assert_eq!(
        node.profile.mount_policy.device_binds,
        strings(expected_device_binds),
        "{test_name} deviceBinds drifted from the runner-shape snapshot"
    );
}

#[test]
fn cloud_hypervisor_runner_shape_matches_rendered_snapshot() {
    assert_role_shape(
        "cloud_hypervisor_runner_shape_matches_rendered_snapshot",
        ProcessRole::CloudHypervisorRunner,
        "corp-full",
        &[
            "microvm@corp-full",
            "--cpus",
            "boot=1",
            "--watchdog",
            "--kernel",
            "/nix/store/<HASH>-linux-6.18.33-dev/vmlinux",
            "--initramfs",
            "/nix/store/<HASH>-initrd-linux-6.18.33/initrd",
            "--cmdline",
            "earlyprintk=ttyS0 console=ttyS0 reboot=t panic=-1 nofb video=off root=fstab loglevel=4 lsm=landlock,yama,bpf init=/nix/store/<HASH>-nixos-system-corp-full-26.05pre-git/init",
            "--seccomp",
            "true",
            "--memory",
            "shared=on,size=512M",
            "--platform",
            "oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]",
            "--console",
            "null",
            "--serial",
            "tty",
            "--vsock",
            "cid=1110,socket=/var/lib/nixling/vms/corp-full/vsock.sock",
            "--gpu",
            "socket=/run/nixling/vms/corp-full/gpu.sock",
            "--fs",
            "socket=/run/nixling/vms/corp-full/ro-store.sock,tag=ro-store",
            "socket=/run/nixling/vms/corp-full/nl-meta.sock,tag=nl-meta",
            "socket=/run/nixling/vms/corp-full/nl-hkeys.sock,tag=nl-hkeys",
            "socket=/run/nixling/vms/corp-full/nl-ssh-host.sock,tag=nl-ssh-host",
            "--api-socket",
            "/var/lib/nixling/vms/corp-full/corp-full.sock",
            "--net",
            "mac=02:76:53:AE:57:0A,tap=work-l10",
            "--tpm",
            "socket=/run/nixling/vms/corp-full/tpm.sock",
            "--vhost-user-media",
            "socket=/run/nixling-video/corp-full/video.sock",
            "--generic-vhost-user",
            "socket=/run/nixling/vms/corp-full/snd.sock,virtio_id=25,queue_sizes=[64,64,64,64]",
        ],
        &["/dev/kvm", "/dev/vhost-net"],
    );
}

#[test]
fn virtiofsd_runner_shape_matches_rendered_snapshot() {
    assert_role_shape(
        "virtiofsd_runner_shape_matches_rendered_snapshot",
        ProcessRole::Virtiofsd,
        "corp-full",
        &[
            "microvm-virtiofsd@corp-full-ro-store",
            "--socket-path=/run/nixling/vms/corp-full/ro-store.sock",
            "--shared-dir=/var/lib/nixling/vms/corp-full/store-view/live",
            "--thread-pool-size",
            "1",
            "--sandbox=chroot",
            "--inode-file-handles=never",
            "--cache=auto",
            "--readonly",
        ],
        &[],
    );
}

#[test]
fn swtpm_runner_shape_matches_rendered_snapshot() {
    assert_role_shape(
        "swtpm_runner_shape_matches_rendered_snapshot",
        ProcessRole::Swtpm,
        "corp-full",
        &[
            "microvm-swtpm@corp-full",
            "socket",
            "--tpmstate",
            "dir=/var/lib/nixling/vms/corp-full/swtpm",
            "--ctrl",
            "type=unixio,path=/run/nixling/vms/corp-full/tpm.sock,mode=0660",
            "--tpm2",
            "--flags",
            "startup-clear",
        ],
        &[],
    );
}
