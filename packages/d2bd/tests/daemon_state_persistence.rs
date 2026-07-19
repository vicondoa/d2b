mod common;

mod daemon_state_persistence {
    use serde_json::{Value, json};
    use std::fs;
    use std::process::{Command, Stdio};

    use super::common::{
        DaemonFixture, complete_component_session_handshake, current_username, spawn_d2bd_serve,
    };

    #[test]
    fn restores_pidfd_table_before_component_session_dispatch() {
        let fixture = DaemonFixture::new("daemon-state-persistence.");
        let username = current_username();
        fixture.write_config(&[&username], &[&username]);
        let report_json = fixture.root().join("state-restore-report.json");
        let pidfd_table_json = fixture.daemon_state_dir.join("pidfd-table.json");
        let runtime_snapshot_json = fixture.daemon_state_dir.join("corp-vm/runtime.ch.json");

        let initial = spawn_d2bd_serve(&fixture, false, None);
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

        let restore = spawn_d2bd_serve(&fixture, true, Some(report_json.as_path()));
        complete_component_session_handshake(&fixture.socket_path);
        let restore_status = restore.wait();
        assert!(
            restore_status.success(),
            "d2bd restore serve exited with {restore_status:?}"
        );

        let report = read_json(&report_json);
        let entries = report["entries"].as_array().expect("report entries array");
        assert_eq!(entries.len(), 1, "daemon state restore report entries");
        assert_eq!(entries[0]["vm"], "corp-vm");
        assert_eq!(entries[0]["roleId"], "ch");
        assert_eq!(entries[0]["outcome"]["outcome"], "adopt");

        let pidfd_table = read_json(&pidfd_table_json);
        assert_eq!(
            pidfd_table["entries"]
                .as_array()
                .expect("pidfd entries")
                .len(),
            1,
            "adopted runner remains supervised across the daemon restart"
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

    fn process_start_time_ticks(pid: u32) -> u64 {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read runner /proc stat");
        stat.split_whitespace()
            .nth(21)
            .expect("starttime field")
            .parse()
            .expect("parse starttime ticks")
    }
}
