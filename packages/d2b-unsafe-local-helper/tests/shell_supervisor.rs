use d2b_contracts::terminal_wire::TerminalStream;
use d2b_contracts::unsafe_local_wire::{
    HelperScopeKind, HelperScopeState, HelperShellPolicy, HelperShellRequest,
    HelperTerminalChunkBase64, HelperTerminalControl, HelperTerminalReadOutput,
    HelperTerminalRequest, HelperTerminalResize, HelperTerminalResponse, HelperTerminalWriteStdin,
    UnsafeLocalHelperToDaemon, decode_unsafe_local_terminal_frame,
    encode_unsafe_local_terminal_frame,
};
use d2b_contracts::{public_wire::ShellName, terminal_wire::TerminalSize};
use d2b_core::base64_codec;
use d2b_core::workload_identity::WorkloadIdentity;
use d2b_realm_core::ids::OperationId;
use d2b_unsafe_local_helper::environment::ManagerEnvironment;
use d2b_unsafe_local_helper::runtime::ScopeRuntime;
use d2b_unsafe_local_helper::systemd::{
    ScopeError, ScopeInspection, UserScopeManager, VerifiedScope,
};
use nix::libc;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

const SUPERVISOR_ID: &str = "0123456789abcdef0123456789abcdef";

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        for _ in 0..32 {
            let mut random = [0u8; 4];
            getrandom::getrandom(&mut random).unwrap();
            let path = Path::new("/tmp").join(format!(
                "d2b-sh-{}-{:08x}",
                std::process::id(),
                u32::from_ne_bytes(random)
            ));
            if fs::create_dir(&path).is_ok() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                return Self { path };
            }
        }
        panic!("could not reserve integration directory");
    }

    fn socket(&self) -> PathBuf {
        self.path.join(format!(".d2b-shell-{SUPERVISOR_ID}.sock"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Supervisor {
    child: Child,
    scratch: Scratch,
}

impl Supervisor {
    fn start() -> Self {
        let scratch = Scratch::new();
        let user = get_user_by_uid(get_current_uid()).expect("passwd identity");
        let home = user.home_dir().to_path_buf();
        let mut environment = std::env::vars()
            .map(|(key, value)| (key, Value::String(value)))
            .collect::<serde_json::Map<String, Value>>();
        environment.insert(
            "D2B_TEST_ENV".to_owned(),
            Value::String("manager-env-canary".to_owned()),
        );
        let spec = json!({
            "supervisorId": SUPERVISOR_ID,
            "runtimeDirectory": scratch.path,
            "environment": environment,
            "cwd": home,
            "initialRows": 24,
            "initialCols": 80,
            "outputRingBytes": 262144
        });
        let encoded = serde_json::to_vec(&spec).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_d2b-unsafe-local-helper"))
            .arg("shell-supervisor")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn supervisor");
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(&(encoded.len() as u32).to_le_bytes())
            .unwrap();
        stdin.write_all(&encoded).unwrap();
        stdin.write_all(&[1]).unwrap();
        drop(stdin);
        let mut ready = [0u8; 1];
        child
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .expect("supervisor ready");
        assert_eq!(ready, [1]);
        assert!(scratch.socket().exists());
        Self { child, scratch }
    }

    fn control(&self, request_id: u64, action: Value) -> (Value, UnixStream) {
        let mut stream = UnixStream::connect(self.scratch.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write_private_frame(
            &mut stream,
            &json!({
                "version": 1,
                "requestId": request_id,
                "action": action
            }),
        );
        let response = read_private_frame(&mut stream);
        assert_eq!(response["version"], 1);
        assert_eq!(response["requestId"], request_id);
        (response, stream)
    }

    fn attach(&self, request_id: u64, force: bool) -> (Value, UnixStream) {
        self.control(
            request_id,
            json!({
                "op": "attach",
                "args": {
                    "force": force,
                    "initialTerminalSize": {"rows": 24, "cols": 80}
                }
            }),
        )
    }

    fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "supervisor did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Clone)]
struct FakeScopeManager {
    environment: ManagerEnvironment,
    active: Arc<Mutex<Option<(u32, VerifiedScope)>>>,
}

impl UserScopeManager for FakeScopeManager {
    fn manager_environment(&self) -> Result<ManagerEnvironment, ScopeError> {
        Ok(self.environment.clone())
    }

    fn start_scope(
        &self,
        supervisor_pid: u32,
        kind: HelperScopeKind,
    ) -> Result<VerifiedScope, ScopeError> {
        if kind != HelperScopeKind::PersistentShell {
            return Err(ScopeError::IdentityMismatch);
        }
        let scope = VerifiedScope {
            unit_name: "d2b-shell-test.scope".to_owned(),
            invocation_id: "00112233445566778899aabbccddeeff".to_owned(),
            control_group: "/user.slice/d2b-shell-test.scope".to_owned(),
            kind,
        };
        *self.active.lock().unwrap() = Some((supervisor_pid, scope.clone()));
        Ok(scope)
    }

    fn inspect_scope(&self, scope: &VerifiedScope) -> Result<ScopeInspection, ScopeError> {
        let active = self.active.lock().unwrap();
        let Some((pid, expected)) = active.as_ref() else {
            return Ok(ScopeInspection {
                state: HelperScopeState::Exited,
                identity_matches: false,
            });
        };
        let identity_matches = expected == scope;
        let state = if Path::new(&format!("/proc/{pid}")).exists() {
            HelperScopeState::Active
        } else {
            HelperScopeState::Exited
        };
        Ok(ScopeInspection {
            state,
            identity_matches,
        })
    }

    fn terminate_scope(&self, scope: &VerifiedScope, signal: i32) -> Result<(), ScopeError> {
        let active = self.active.lock().unwrap();
        let Some((pid, expected)) = active.as_ref() else {
            return Ok(());
        };
        if expected != scope {
            return Err(ScopeError::IdentityMismatch);
        }
        let signal = Signal::try_from(signal).map_err(|_| ScopeError::StopFailed)?;
        match kill(Pid::from_raw(*pid as i32), Some(signal)) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(_) => Err(ScopeError::StopFailed),
        }
    }

    fn stop_scope(&self, scope: &VerifiedScope) -> Result<(), ScopeError> {
        self.terminate_scope(scope, libc::SIGKILL)
    }
}

#[test]
fn real_supervisor_preserves_pty_across_reconnect_and_kills_exact_scope() {
    let mut unrelated = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn unrelated same-uid process");
    let mut supervisor = Supervisor::start();

    let (attached, mut terminal) = supervisor.attach(1, false);
    assert_eq!(attached["result"]["kind"], "attached");
    assert_eq!(attached["result"]["value"]["forceEvicted"], false);
    terminal
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    terminal
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let home = get_user_by_uid(get_current_uid())
        .unwrap()
        .home_dir()
        .to_path_buf();
    let command = b"if test -t 0; then printf 'D2B_RESULT:%s:%s:tty\\n' \"$D2B_TEST_ENV\" \"$PWD\"; else printf 'D2B_RESULT:notty\\n'; fi\n";
    let write = HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
        request_id: 10,
        offset: 0,
        chunk_base64: HelperTerminalChunkBase64::new(base64_codec::encode(command)).unwrap(),
        eof: false,
    });
    write_terminal_frame(&mut terminal, &write);
    assert!(matches!(
        read_terminal_frame(&mut terminal),
        HelperTerminalResponse::WriteStdin(_)
    ));
    let expected = format!(
        "D2B_RESULT:manager-env-canary:{}:tty",
        home.to_string_lossy()
    );
    let (mut cursor, output) = read_until(&mut terminal, 11, 0, expected.as_bytes());
    assert!(
        output
            .windows(expected.len())
            .any(|window| window == expected.as_bytes()),
        "login shell did not inherit manager environment, passwd cwd, and PTY"
    );

    write_terminal_frame(
        &mut terminal,
        &HelperTerminalRequest::Resize(HelperTerminalResize {
            request_id: 12,
            control_sequence: 1,
            rows: 41,
            cols: 101,
        }),
    );
    assert!(matches!(
        read_terminal_frame(&mut terminal),
        HelperTerminalResponse::Resize(_)
    ));
    let geometry_command = b"stty size\n";
    write_terminal_frame(
        &mut terminal,
        &HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
            request_id: 13,
            offset: command.len() as u64,
            chunk_base64: HelperTerminalChunkBase64::new(base64_codec::encode(geometry_command))
                .unwrap(),
            eof: false,
        }),
    );
    let _ = read_terminal_frame(&mut terminal);
    let (next_cursor, geometry) = read_until(&mut terminal, 14, cursor, b"41 101");
    cursor = next_cursor;
    assert!(geometry.windows(6).any(|window| window == b"41 101"));

    write_terminal_frame(
        &mut terminal,
        &HelperTerminalRequest::CloseAttachment(HelperTerminalControl {
            request_id: 15,
            control_sequence: 2,
        }),
    );
    assert!(matches!(
        read_terminal_frame(&mut terminal),
        HelperTerminalResponse::CloseAttachment(_)
    ));
    drop(terminal);

    let (status, _) = supervisor.control(2, json!({"op": "status"}));
    assert_eq!(status["result"]["kind"], "status");
    assert_eq!(status["result"]["value"]["running"], true);
    assert_eq!(status["result"]["value"]["attached"], false);

    let (_, old_terminal) = supervisor.attach(3, false);
    let (rejected, _) = supervisor.attach(4, false);
    assert_eq!(rejected["result"]["kind"], "rejected");
    assert_eq!(rejected["result"]["value"]["code"], "already-attached");
    let (forced, _new_terminal) = supervisor.attach(5, true);
    assert_eq!(forced["result"]["kind"], "attached");
    assert_eq!(forced["result"]["value"]["forceEvicted"], true);
    old_terminal
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut byte = [0u8; 1];
    assert!(old_terminal.take_error().unwrap().is_none());
    assert!(matches!((&old_terminal).read(&mut byte), Ok(0) | Err(_)));

    let (kill, _) = supervisor.control(6, json!({"op": "kill"}));
    assert_eq!(kill["result"]["kind"], "killAccepted");
    supervisor.wait_for_exit();
    assert!(!supervisor.scratch.socket().exists());
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "shell kill affected unrelated same-uid process"
    );
    let _ = unrelated.kill();
    let _ = unrelated.wait();

    let _ = cursor;
}

#[test]
fn helper_runtime_creates_persists_and_reconstructs_real_supervisor() {
    let scratch = Scratch::new();
    exercise_helper_runtime_reconstruction(&scratch, "single");
}

#[test]
fn repeated_missing_socket_kill_cleans_scope_ledger() {
    let scratch = Scratch::new();
    for iteration in 0..8 {
        exercise_helper_runtime_reconstruction(&scratch, &format!("stress-{iteration}"));
    }
}

fn exercise_helper_runtime_reconstruction(scratch: &Scratch, operation_suffix: &str) {
    if get_current_uid() == 0 {
        return;
    }
    let user = get_user_by_uid(get_current_uid()).unwrap();
    let mut environment = BTreeMap::from([
        (
            "PATH".to_owned(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned()),
        ),
        (
            "HOME".to_owned(),
            user.home_dir().to_string_lossy().into_owned(),
        ),
    ]);
    environment.insert(
        "XDG_RUNTIME_DIR".to_owned(),
        scratch.path.display().to_string(),
    );
    environment.insert("D2B_TEST_ENV".to_owned(), "manager-env-canary".to_owned());
    environment.insert("TERM".to_owned(), "dumb".to_owned());
    environment.insert("COLORTERM".to_owned(), "manager-value".to_owned());
    let manager = FakeScopeManager {
        environment: ManagerEnvironment::parse(
            environment
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect(),
        )
        .unwrap(),
        active: Arc::new(Mutex::new(None)),
    };
    let ledger = scratch.path.join("ledger.json");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_d2b-unsafe-local-helper"));
    let runtime = ScopeRuntime::with_paths_and_executable(
        manager.clone(),
        user.home_dir().to_path_buf(),
        ledger.clone(),
        binary.clone(),
    )
    .unwrap();
    let workload = workload();
    let create_operation = format!("op-runtime-create-{operation_suffix}");
    let created = runtime
        .shell(attach_request(&create_operation, workload.clone(), false))
        .unwrap();
    let (frame, fd) = created.into_parts();
    assert!(matches!(frame, UnsafeLocalHelperToDaemon::TerminalReady(_)));
    let mut terminal = UnixStream::from(fd.expect("terminal fd"));
    terminal
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let command = b"printf 'D2B_RUNTIME_ENV:%s:%s\\n' \"$TERM\" \"$COLORTERM\"; if test -n \"$BASH_VERSION\"; then case $- in *i*) d2b_mode=interactive ;; *) d2b_mode=noninteractive ;; esac; if shopt -q login_shell; then d2b_login=login; else d2b_login=nonlogin; fi; printf 'D2B_RUNTIME_BASH:%s:%s\\n' \"$d2b_mode\" \"$d2b_login\"; fi\n";
    write_terminal_frame(
        &mut terminal,
        &HelperTerminalRequest::WriteStdin(HelperTerminalWriteStdin {
            request_id: 1,
            offset: 0,
            chunk_base64: HelperTerminalChunkBase64::new(base64_codec::encode(command)).unwrap(),
            eof: false,
        }),
    );
    let _ = read_terminal_frame(&mut terminal);
    let bash_login_shell = user.shell().file_name().and_then(|name| name.to_str()) == Some("bash");
    let needle = if bash_login_shell {
        b"D2B_RUNTIME_BASH:interactive:login".as_slice()
    } else {
        b"D2B_RUNTIME_ENV:xterm-256color:truecolor".as_slice()
    };
    let (_, output) = read_until(&mut terminal, 2, 0, needle);
    assert!(
        output
            .windows(b"D2B_RUNTIME_ENV:xterm-256color:truecolor".len())
            .any(|window| window == b"D2B_RUNTIME_ENV:xterm-256color:truecolor"),
        "persistent shell inherited non-terminal manager TERM or COLORTERM"
    );
    if bash_login_shell {
        assert!(
            output
                .windows(b"D2B_RUNTIME_BASH:interactive:login".len())
                .any(|window| window == b"D2B_RUNTIME_BASH:interactive:login"),
            "Bash persistent shell was not interactive and login-mode"
        );
    }
    drop(terminal);

    let reconstructed = ScopeRuntime::with_paths_and_executable(
        manager,
        user.home_dir().to_path_buf(),
        ledger,
        binary,
    )
    .unwrap();
    let snapshot = reconstructed.snapshot(11).unwrap();
    assert_eq!(snapshot.scopes.len(), 1);
    assert!(snapshot.scopes[0].persistent_shell.is_some());
    let reattach_operation = format!("op-runtime-reattach-{operation_suffix}");
    let reattached = reconstructed
        .shell(attach_request(&reattach_operation, workload.clone(), true))
        .unwrap();
    let (_, fd) = reattached.into_parts();
    drop(fd);

    let socket = shell_socket(&scratch.path).expect("supervisor socket");
    fs::remove_file(socket).unwrap();
    assert!(
        !has_shell_socket(&scratch.path),
        "missing-socket kill precondition was not established"
    );
    let kill_operation = format!("op-runtime-kill-{operation_suffix}");
    let killed = reconstructed
        .shell(HelperShellRequest::Kill {
            request_id: 4,
            operation_id: OperationId::parse(kill_operation).unwrap(),
            workload,
            policy: shell_policy(),
            name: ShellName::new("host").unwrap(),
        })
        .unwrap();
    assert!(matches!(
        killed.into_parts().0,
        UnsafeLocalHelperToDaemon::Shell(_)
    ));
    assert!(reconstructed.snapshot(12).unwrap().scopes.is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    while has_shell_socket(&scratch.path) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !has_shell_socket(&scratch.path),
        "shell supervisor socket survived the cleanup deadline"
    );
}

fn workload() -> WorkloadIdentity {
    serde_json::from_value(json!({
        "workloadId": "tools",
        "realmId": "host",
        "realmPath": ["host"],
        "canonicalTarget": "tools.host.d2b"
    }))
    .unwrap()
}

fn shell_policy() -> HelperShellPolicy {
    HelperShellPolicy {
        default_name: ShellName::new("host").unwrap(),
        max_sessions: 2,
    }
}

fn attach_request(
    operation_id: &str,
    workload: WorkloadIdentity,
    force: bool,
) -> HelperShellRequest {
    HelperShellRequest::Attach {
        request_id: 1,
        operation_id: OperationId::parse(operation_id).unwrap(),
        workload,
        policy: shell_policy(),
        name: Some(ShellName::new("host").unwrap()),
        force,
        initial_terminal_size: TerminalSize { rows: 24, cols: 80 },
    }
}

fn has_shell_socket(directory: &Path) -> bool {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_type()
                .map(|file_type| file_type.is_socket())
                .unwrap_or(false)
        })
}

fn shell_socket(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .find_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_socket())
                .map(|_| entry.path())
        })
}

fn write_private_frame(stream: &mut UnixStream, value: &Value) {
    let body = serde_json::to_vec(value).unwrap();
    stream
        .write_all(&(body.len() as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
}

fn read_private_frame(stream: &mut UnixStream) -> Value {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    let length = u32::from_le_bytes(prefix) as usize;
    assert!(length <= 16 * 1024);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn write_terminal_frame(stream: &mut UnixStream, request: &HelperTerminalRequest) {
    let frame = encode_unsafe_local_terminal_frame(request).unwrap();
    stream.write_all(&frame).unwrap();
}

fn read_terminal_frame(stream: &mut UnixStream) -> HelperTerminalResponse {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    let length = u32::from_le_bytes(prefix) as usize;
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    stream.read_exact(&mut frame[4..]).unwrap();
    decode_unsafe_local_terminal_frame(&frame).unwrap()
}

fn read_until(
    stream: &mut UnixStream,
    mut request_id: u64,
    mut cursor: u64,
    needle: &[u8],
) -> (u64, Vec<u8>) {
    let mut output = Vec::new();
    for _ in 0..8 {
        write_terminal_frame(
            stream,
            &HelperTerminalRequest::ReadOutput(HelperTerminalReadOutput {
                request_id,
                stream: TerminalStream::Stdout,
                cursor,
                max_len: 65_536,
                wait: true,
                timeout_ms: 1_000,
            }),
        );
        let HelperTerminalResponse::ReadOutput(response) = read_terminal_frame(stream) else {
            panic!("unexpected terminal response");
        };
        let data = base64_codec::decode(response.result.data_base64.as_str()).unwrap();
        output.extend_from_slice(&data);
        cursor = response.result.next_cursor;
        if output.windows(needle.len()).any(|window| window == needle) {
            return (cursor, output);
        }
        request_id += 1;
    }
    (cursor, output)
}
