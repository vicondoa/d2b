mod common;

mod daemon_state_persistence {
    use std::fs;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::common::{
        DaemonFixture, HELLO_FRAME, TestPeer, last_non_empty_line, spawn_nixlingd_serve,
        test_client,
    };

    const VM_STOP_FRAME: &str = r#"{"type":"vmStop","vm":"corp-vm","apply":true,"json":true}"#;

    #[test]
    fn restores_pidfd_table_and_clears_after_vm_stop() {
        let fixture = DaemonFixture::new("daemon-state-persistence.");
        fixture.write_config(&["launcher-user"], &["admin-user", "launcher-user"]);
        let report_json = fixture.root().join("state-restore-report.json");
        let stop_response_json = fixture.root().join("vm-stop-response.json");
        let pidfd_table_json = fixture.daemon_state_dir.join("pidfd-table.json");
        let runtime_snapshot_json = fixture.daemon_state_dir.join("corp-vm/runtime.ch.json");

        let initial = spawn_nixlingd_serve(&fixture, &TestPeer::launcher(), false, None);
        initial.kill_and_wait();
        fixture.reset_runtime_endpoints();

        let runner = OrphanProcess::spawn_sleep();
        let runner_pid = runner.pid;
        let start_time_ticks = process_start_time_ticks(runner_pid);

        fs::create_dir_all(fixture.daemon_state_dir.join("corp-vm")).expect("create VM state dir");
        write_json(
            &pidfd_table_json,
            &json!({
                "entries": [{
                    "vm": "corp-vm",
                    "role": "ch-runner",
                    "pid": runner_pid,
                    "startTimeTicks": start_time_ticks
                }]
            }),
        );
        write_json(
            &runtime_snapshot_json,
            &json!({
                "vm": "corp-vm",
                "roleId": "ch",
                "role": "cloud-hypervisor",
                "pid": runner_pid,
                "startTimeTicks": start_time_ticks,
                "snapshottedAt": "2026-01-01T00:00:00Z"
            }),
        );

        let restore = spawn_nixlingd_serve(
            &fixture,
            &TestPeer::launcher(),
            true,
            Some(report_json.as_path()),
        );
        let (rc, client_output) = test_client(&fixture.socket_path, &[HELLO_FRAME, VM_STOP_FRAME]);
        assert_eq!(rc, 0, "vm stop client exit code; output:\n{client_output}");
        fs::write(&stop_response_json, last_non_empty_line(&client_output))
            .expect("write stop response");
        let restore_status = restore.wait();
        assert!(
            restore_status.success(),
            "nixlingd restore serve exited with {restore_status:?}"
        );

        let report = read_json(&report_json);
        let entries = report["entries"].as_array().expect("report entries array");
        assert_eq!(entries.len(), 1, "daemon state restore report entries");
        assert_eq!(entries[0]["vm"], "corp-vm");
        assert_eq!(entries[0]["roleId"], "ch");
        assert_eq!(entries[0]["outcome"]["outcome"], "adopt");

        let stop_response = read_json(&stop_response_json);
        assert_eq!(stop_response["type"], "mutatingVerbResponse");
        assert_eq!(stop_response["verb"], "vm stop");
        assert_eq!(stop_response["outcome"], "applied");
        assert_eq!(
            stop_response["summary"],
            "vm stop corp-vm: drained 1 pidfd_table entry in reverse DAG order (ch-runner)"
        );

        assert!(
            wait_for_pid_absent(runner_pid, Duration::from_secs(5)),
            "restored runner pid is still alive after vm stop"
        );

        let pidfd_table = read_json(&pidfd_table_json);
        assert_eq!(
            pidfd_table["entries"]
                .as_array()
                .expect("pidfd entries")
                .len(),
            0,
            "pidfd-table snapshot cleared after stop"
        );
    }

    struct OrphanProcess {
        pid: u32,
    }

    impl OrphanProcess {
        fn spawn_sleep() -> Self {
            let output = Command::new("sh")
                .arg("-c")
                .arg("sleep 120 >/dev/null 2>&1 & echo $!")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .expect("spawn orphan sleep launcher");
            assert!(
                output.status.success(),
                "orphan sleep launcher exited with {:?}",
                output.status
            );
            let stdout = String::from_utf8(output.stdout).expect("sleep pid stdout is UTF-8");
            let pid = stdout.trim().parse().expect("parse sleep pid");
            Self { pid }
        }
    }

    impl Drop for OrphanProcess {
        fn drop(&mut self) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(self.pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    }

    fn write_json(path: &std::path::Path, value: &Value) {
        fs::write(
            path,
            serde_json::to_vec_pretty(value).expect("serialize test JSON"),
        )
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    fn read_json(path: &std::path::Path) -> Value {
        let bytes = fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
    }

    fn wait_for_pid_absent(pid: u32, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let proc_path = format!("/proc/{pid}");
        while std::time::Instant::now() < deadline {
            if !std::path::Path::new(&proc_path).exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        !std::path::Path::new(&proc_path).exists()
    }

    fn process_start_time_ticks(pid: u32) -> u64 {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read runner /proc stat");
        stat.split_whitespace()
            .nth(21)
            .expect("starttime field")
            .parse()
            .expect("parse starttime ticks")
    }
}
