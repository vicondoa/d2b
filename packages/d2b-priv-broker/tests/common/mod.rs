#![allow(dead_code)]

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use tempfile::{Builder, TempDir};

const BROKER_BIN: &str = env!("CARGO_BIN_EXE_d2b-priv-broker");
const O_APPEND: u32 = 0o2000;

pub const D2BD_UID: u32 = 4242;

pub struct Scratch {
    inner: Option<TempDir>,
}

impl Scratch {
    pub fn new(prefix: &str) -> Self {
        let inner = Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("create broker test scratch dir");
        Self { inner: Some(inner) }
    }

    pub fn path(&self) -> &Path {
        self.inner.as_ref().expect("scratch dir still alive").path()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            drop(inner);
        }
    }
}

pub struct TestBroker {
    scratch: Scratch,
    child: Child,
    socket_path: PathBuf,
    audit_dir: PathBuf,
    server_log_path: PathBuf,
    d2bd_uid: u32,
}

impl TestBroker {
    pub fn spawn(prefix: &str) -> Self {
        let scratch = Scratch::new(prefix);
        let run_dir = scratch.path().join("run/d2b");
        let audit_dir = scratch.path().join("var/lib/d2b/audit");
        fs::create_dir_all(&run_dir).expect("create broker run dir");
        fs::create_dir_all(&audit_dir).expect("create broker audit dir");

        let socket_path = run_dir.join("priv.sock");
        let server_log_path = scratch.path().join("server.log");
        let server_log = File::create(&server_log_path).expect("create broker server log");
        let server_log_err = server_log.try_clone().expect("clone broker server log");
        let current_gid = nix::unistd::Gid::current().as_raw();

        let child = Command::new(BROKER_BIN)
            .arg("serve")
            .arg("--socket-path")
            .arg(&socket_path)
            .arg("--audit-dir")
            .arg(&audit_dir)
            .arg("--d2bd-uid")
            .arg(D2BD_UID.to_string())
            .arg("--d2bd-gid")
            .arg(current_gid.to_string())
            .arg("--test-mode")
            .env("RUST_LOG", "off")
            .stdout(Stdio::from(server_log))
            .stderr(Stdio::from(server_log_err))
            .spawn()
            .expect("spawn d2b-priv-broker serve");

        let broker = Self {
            scratch,
            child,
            socket_path,
            audit_dir,
            server_log_path,
            d2bd_uid: D2BD_UID,
        };
        broker.wait_for_socket();
        broker
    }

    pub fn socket_mode(&self) -> u32 {
        fs::metadata(&self.socket_path)
            .expect("stat broker socket")
            .permissions()
            .mode()
            & 0o777
    }

    pub fn scratch_path(&self) -> &Path {
        self.scratch.path()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn d2bd_uid(&self) -> u32 {
        self.d2bd_uid
    }

    pub fn server_log(&self) -> String {
        fs::read_to_string(&self.server_log_path).unwrap_or_default()
    }

    pub fn audit_path(&self) -> PathBuf {
        let mut paths: Vec<PathBuf> = fs::read_dir(&self.audit_dir)
            .expect("read broker audit dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("broker-") && name.ends_with(".jsonl"))
            })
            .collect();
        paths.sort();
        assert_eq!(paths.len(), 1, "expected exactly one daily audit file");
        paths.remove(0)
    }

    pub fn audit_contents(&self) -> String {
        fs::read_to_string(self.audit_path()).expect("read broker audit file")
    }

    pub fn probe_hello(&self, uid: u32) -> ProbeOutput {
        self.run_probe([
            OsStr::new("probe-hello"),
            OsStr::new("--socket-path"),
            self.socket_path.as_os_str(),
            OsStr::new("--test-uid"),
            OsStr::new(&uid.to_string()),
        ])
    }

    pub fn probe_stub(&self, uid: u32, operation: &str) -> ProbeOutput {
        self.run_probe([
            OsStr::new("probe-stub"),
            OsStr::new("--socket-path"),
            self.socket_path.as_os_str(),
            OsStr::new("--test-uid"),
            OsStr::new(&uid.to_string()),
            OsStr::new("--operation"),
            OsStr::new(operation),
        ])
    }

    pub fn probe_export_audit(&self, uid: u32, caller_role: &str) -> ProbeOutput {
        self.run_probe([
            OsStr::new("probe-export-audit"),
            OsStr::new("--socket-path"),
            self.socket_path.as_os_str(),
            OsStr::new("--test-uid"),
            OsStr::new(&uid.to_string()),
            OsStr::new("--caller-role"),
            OsStr::new(caller_role),
        ])
    }

    pub fn audit_write_fds(&self, audit_path: &Path) -> Vec<FdFlags> {
        let fd_dir = PathBuf::from(format!("/proc/{}/fd", self.pid()));
        let fdinfo_dir = PathBuf::from(format!("/proc/{}/fdinfo", self.pid()));
        let mut write_fds = Vec::new();
        for entry in fs::read_dir(&fd_dir).expect("read broker fd dir") {
            let entry = entry.expect("read broker fd entry");
            let target = match fs::read_link(entry.path()) {
                Ok(target) => target,
                Err(_) => continue,
            };
            if target != audit_path {
                continue;
            }
            let fd = entry.file_name().to_string_lossy().into_owned();
            let fdinfo = fs::read_to_string(fdinfo_dir.join(&fd)).expect("read broker fdinfo");
            let flags_raw = fdinfo
                .lines()
                .find_map(|line| line.strip_prefix("flags:\t"))
                .or_else(|| fdinfo.lines().find_map(|line| line.strip_prefix("flags:")))
                .map(str::trim)
                .expect("fdinfo flags line")
                .to_owned();
            let flags = u32::from_str_radix(&flags_raw, 8).expect("octal fd flags");
            if flags & 3 != 0 {
                write_fds.push(FdFlags {
                    fd,
                    flags_raw,
                    flags,
                });
            }
        }
        write_fds
    }

    fn wait_for_socket(&self) {
        for _ in 0..50 {
            if is_socket(&self.socket_path) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "broker did not create {}; server log:\n{}",
            self.socket_path.display(),
            self.server_log()
        );
    }

    fn run_probe<I, S>(&self, args: I) -> ProbeOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        ProbeOutput(
            Command::new(BROKER_BIN)
                .args(args)
                .output()
                .expect("run broker probe command"),
        )
    }
}

impl Drop for TestBroker {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            Err(_) => {}
        }
    }
}

pub struct ProbeOutput(Output);

impl ProbeOutput {
    pub fn assert_success(&self) {
        assert!(
            self.0.status.success(),
            "expected success, got status {:?}\nstdout:\n{}\nstderr:\n{}",
            self.0.status.code(),
            self.stdout(),
            self.stderr()
        );
    }

    pub fn assert_exit_code(&self, code: i32) {
        assert_eq!(
            self.0.status.code(),
            Some(code),
            "unexpected exit status\nstdout:\n{}\nstderr:\n{}",
            self.stdout(),
            self.stderr()
        );
    }

    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.0.stdout).into_owned()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.0.stderr).into_owned()
    }
}

#[derive(Debug)]
pub struct FdFlags {
    pub fd: String,
    pub flags_raw: String,
    pub flags: u32,
}

impl FdFlags {
    pub fn is_append_only(&self) -> bool {
        self.flags & O_APPEND != 0
    }
}

pub fn audit_file_metadata(path: &Path) -> io::Result<(u32, u32, u32)> {
    let metadata = fs::metadata(path)?;
    Ok((
        metadata.uid(),
        metadata.gid(),
        metadata.permissions().mode() & 0o777,
    ))
}

fn is_socket(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}
