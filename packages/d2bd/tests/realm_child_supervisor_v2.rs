use std::io::Read as _;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use d2b_host::realm_children::RealmChildRole;
use d2bd::realm_child_supervisor::{
    ProcRealmChildAdoptionVerifier, RealmChildAdoptionCandidate, RealmChildAdoptionPair,
    RealmChildAdoptionVerifier, RealmChildHandle, RealmChildPair, RealmChildSupervisor,
    RealmChildSupervisorError,
};
use sha2::{Digest as _, Sha256};

struct Children(Vec<Child>);

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn pidfd(pid: u32) -> std::os::fd::OwnedFd {
    rustix::process::pidfd_open(
        rustix::process::Pid::from_raw(pid as i32).unwrap(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap()
}

fn spawn_pair() -> (Children, RealmChildPair) {
    let controller = Command::new("sleep").arg("30").spawn().unwrap();
    let broker = Command::new("sleep").arg("30").spawn().unwrap();
    let controller_pid = controller.id();
    let broker_pid = broker.id();
    let root = PathBuf::from("/sys/fs/cgroup/d2b.slice/r-work");
    (
        Children(vec![controller, broker]),
        RealmChildPair {
            realm_id: "work".into(),
            controller: RealmChildHandle {
                role: RealmChildRole::Controller,
                process_id: "controller-1".into(),
                pid: controller_pid,
                pidfd: pidfd(controller_pid),
                executable: PathBuf::from("/bin/sleep"),
                executable_digest: [1; 32],
                controller_generation_id: "generation-1".into(),
                cgroup_leaf: root.join("controller"),
            },
            broker: RealmChildHandle {
                role: RealmChildRole::Broker,
                process_id: "broker-1".into(),
                pid: broker_pid,
                pidfd: pidfd(broker_pid),
                executable: PathBuf::from("/bin/sleep"),
                executable_digest: [2; 32],
                controller_generation_id: "generation-1".into(),
                cgroup_leaf: root.join("broker"),
            },
        },
    )
}

#[test]
fn registers_only_complete_correlated_pairs() {
    let (_children, pair) = spawn_pair();
    let mut supervisor = RealmChildSupervisor::default();
    supervisor.register_pair(pair).unwrap();
    assert_eq!(supervisor.len(), 1);
    assert!(supervisor.get("work").is_some());
}

#[test]
fn rejects_duplicate_realm_without_replacing_authority() {
    let (_first_children, first) = spawn_pair();
    let (_second_children, second) = spawn_pair();
    let mut supervisor = RealmChildSupervisor::default();
    supervisor.register_pair(first).unwrap();
    assert_eq!(
        supervisor.register_pair(second).unwrap_err(),
        RealmChildSupervisorError::DuplicateRealm
    );
    assert_eq!(supervisor.len(), 1);
}

struct PidfdVerifier;

impl RealmChildAdoptionVerifier for PidfdVerifier {
    fn verify(
        &self,
        candidate: &RealmChildAdoptionCandidate,
        pidfd: BorrowedFd<'_>,
    ) -> Result<(), RealmChildSupervisorError> {
        let fdinfo = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))
            .map_err(|_| RealmChildSupervisorError::ProcessMissing)?;
        let pinned_pid = fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .and_then(|value| value.trim().parse::<u32>().ok());
        if pinned_pid == Some(candidate.pid) {
            Ok(())
        } else {
            Err(RealmChildSupervisorError::InvalidPair)
        }
    }
}

#[test]
fn verified_adoption_pins_each_process_before_identity_verification() {
    let (children, _) = spawn_pair();
    let controller_pid = children.0[0].id();
    let broker_pid = children.0[1].id();
    let root = PathBuf::from("/sys/fs/cgroup/d2b.slice/r-work");
    let candidate = RealmChildAdoptionPair {
        realm_id: "work".into(),
        controller: RealmChildAdoptionCandidate {
            role: RealmChildRole::Controller,
            process_id: "controller-1".into(),
            pid: controller_pid,
            executable: PathBuf::from("/bin/sleep"),
            executable_digest: [1; 32],
            controller_generation_id: "generation-1".into(),
            cgroup_leaf: root.join("controller"),
        },
        broker: RealmChildAdoptionCandidate {
            role: RealmChildRole::Broker,
            process_id: "broker-1".into(),
            pid: broker_pid,
            executable: PathBuf::from("/bin/sleep"),
            executable_digest: [2; 32],
            controller_generation_id: "generation-1".into(),
            cgroup_leaf: root.join("broker"),
        },
    };
    let mut supervisor = RealmChildSupervisor::default();
    supervisor.adopt_pair(candidate, &PidfdVerifier).unwrap();
    assert_eq!(supervisor.len(), 1);
}

#[test]
fn proc_verifier_fails_closed_on_executable_mismatch() {
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let _children = Children(vec![child]);
    let candidate = RealmChildAdoptionCandidate {
        role: RealmChildRole::Controller,
        process_id: "controller-1".into(),
        pid,
        executable: PathBuf::from("/bin/false"),
        executable_digest: [1; 32],
        controller_generation_id: "generation-1".into(),
        cgroup_leaf: PathBuf::from("/sys/fs/cgroup/d2b.slice/r-work/controller"),
    };
    let process_pidfd = pidfd(pid);
    assert_eq!(
        ProcRealmChildAdoptionVerifier
            .verify(&candidate, process_pidfd.as_fd())
            .unwrap_err(),
        RealmChildSupervisorError::ExecutableMismatch
    );
}

#[test]
fn proc_verifier_accepts_the_pinned_process_identity() {
    let mut child = Command::new("sh")
        .args(["-c", "printf ready; read _"])
        .env("D2B_CONTROLLER_GENERATION", "generation-1")
        .env("D2B_PROCESS_ID", "controller-1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut ready = [0_u8; 5];
    child
        .stdout
        .as_mut()
        .expect("child readiness pipe")
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(&ready, b"ready");
    let pid = child.id();
    let _children = Children(vec![child]);
    let proc_root = PathBuf::from("/proc").join(pid.to_string());
    let executable = std::fs::read_link(proc_root.join("exe")).unwrap();
    let executable_digest: [u8; 32] = Sha256::digest(std::fs::read(proc_root.join("exe")).unwrap())
        .as_slice()
        .try_into()
        .unwrap();
    let cgroup_path = std::fs::read_to_string(proc_root.join("cgroup"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap()
        .trim_start_matches('/')
        .to_owned();
    let candidate = RealmChildAdoptionCandidate {
        role: RealmChildRole::Controller,
        process_id: "controller-1".into(),
        pid,
        executable,
        executable_digest,
        controller_generation_id: "generation-1".into(),
        cgroup_leaf: PathBuf::from("/sys/fs/cgroup").join(cgroup_path),
    };
    let process_pidfd = pidfd(pid);

    ProcRealmChildAdoptionVerifier
        .verify(&candidate, process_pidfd.as_fd())
        .unwrap();
}
