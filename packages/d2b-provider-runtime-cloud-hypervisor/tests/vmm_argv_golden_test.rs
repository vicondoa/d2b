use d2b_provider_runtime_cloud_hypervisor::{ChArgvInput, ChNetIface, ChVsock, generate_ch_argv};

fn input() -> ChArgvInput {
    ChArgvInput {
        vm_name: "corp-vm".to_owned(),
        ch_binary_path: "/nix/store/cloud-hypervisor/bin/cloud-hypervisor".to_owned(),
        cpus: 2,
        watchdog: true,
        kernel_path: "/nix/store/kernel/vmlinux".to_owned(),
        initramfs_path: Some("/nix/store/initrd".to_owned()),
        cmdline: "console=ttyS0".to_owned(),
        seccomp: "true".to_owned(),
        memory: "shared=on,size=512M".to_owned(),
        platform_oem_strings: Vec::new(),
        console: "null".to_owned(),
        serial: "tty".to_owned(),
        primary_vsock: Some(ChVsock {
            cid: 14,
            socket: "notify.vsock".to_owned(),
        }),
        extra_vsock: Vec::new(),
        fs_shares: Vec::new(),
        api_socket_path: "ch-api.sock".to_owned(),
        net_ifaces: vec![ChNetIface {
            mac: "02:00:00:00:00:01".to_owned(),
            tap_fd: 7,
        }],
        extra_args: vec!["--pvpanic".to_owned()],
    }
}

#[test]
fn argv_preserves_broker_owned_order_and_tap_fd_shape() {
    let input = input();
    let argv = generate_ch_argv(&input).unwrap();
    let joined = argv.join(" ");
    assert!(joined.contains("--vsock cid=14,socket=notify.vsock"));
    assert!(joined.contains("--net fd=7,mac=02:00:00:00:00:01"));
    assert!(joined.ends_with("--pvpanic"));
    assert!(!format!("{:?}", input).contains("/nix/store"));
}

#[test]
fn tap_fd_rejects_stdio_and_accepts_first_inherited_slot() {
    for tap_fd in [-1, 0, 1, 2] {
        let mut input = input();
        input.net_ifaces[0].tap_fd = tap_fd;
        assert!(generate_ch_argv(&input).is_err(), "accepted fd {tap_fd}");
    }

    let mut input = input();
    input.net_ifaces[0].tap_fd = 3;
    assert!(generate_ch_argv(&input).is_ok());
}
