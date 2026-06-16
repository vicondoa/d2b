mod common;

mod daemon_state_lock {
    use std::fs;
    use std::os::unix::fs::MetadataExt as _;
    use std::time::Duration;

    use super::common::{
        assert_contains, run_lock_only, spawn_lock_only, wait_for_file, DaemonFixture,
    };

    fn fixture() -> DaemonFixture {
        let fixture = DaemonFixture::new("daemon-state-lock.");
        fixture.write_config(&[], &[]);
        fixture
    }

    #[test]
    fn lock_file_created_with_current_user_0640() {
        let fixture = fixture();
        let first = spawn_lock_only(&fixture.config_path, &fixture.state_lock_path, 20);
        wait_for_file(&fixture.state_lock_path, Duration::from_secs(15));

        let metadata = fs::symlink_metadata(&fixture.state_lock_path).expect("stat state lock");
        assert_eq!(metadata.mode() & 0o777, 0o640, "state-lock mode");
        assert_eq!(
            metadata.uid(),
            rustix::process::getuid().as_raw(),
            "state-lock uid"
        );
        assert_eq!(
            metadata.gid(),
            rustix::process::getgid().as_raw(),
            "state-lock gid"
        );

        first.kill_and_wait();
    }

    #[test]
    fn second_lock_holder_exits_already_running() {
        let fixture = fixture();
        let first = spawn_lock_only(&fixture.config_path, &fixture.state_lock_path, 20);
        wait_for_file(&fixture.state_lock_path, Duration::from_secs(15));

        let (rc, output) = run_lock_only(
            &fixture.config_path,
            &fixture.state_lock_path,
            &fixture.locks_dir,
        );
        assert_eq!(rc, 41, "second daemon exits AlreadyRunning");
        assert_contains(
            &output,
            "internal-already-running",
            "typed already-running error",
        );

        first.kill_and_wait();
    }

    #[test]
    fn symlink_lock_parent_is_rejected() {
        let fixture = fixture();
        let real_parent = fixture.root().join("real-parent");
        let symlink_parent = fixture.root().join("symlink-parent");
        fs::create_dir_all(&real_parent).expect("create real parent");
        std::os::unix::fs::symlink(&real_parent, &symlink_parent).expect("create symlink parent");

        let (rc, output) = run_lock_only(
            &fixture.config_path,
            &symlink_parent.join("daemon.lock"),
            &fixture.locks_dir,
        );
        assert_eq!(rc, 42, "symlink parent fails closed");
        assert_contains(
            &output,
            "internal-lock-parent-invalid",
            "typed lock-parent error",
        );
        assert_contains(
            &output,
            "must not be a symlink",
            "symlink rejection message",
        );
    }
}
