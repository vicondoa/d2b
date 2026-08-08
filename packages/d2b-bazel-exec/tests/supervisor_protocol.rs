use std::{
    fs::{self, File},
    io::Read,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        OnceLock,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use command_fds::{CommandFdExt, FdMapping};
use nix::{
    fcntl::OFlag,
    sys::signal::{self, SigSet},
    unistd::pipe2,
};

use d2b_bazel_exec::{
    CHILD_STAGE_CODES, ChildIdentity, ContainmentBackend, ExecErrorRecord, ExecStop, GroupIdentity,
    InitialStop, PTRACE_EVENT_EXEC, ProtocolError, RUST_PARENT_STAGE_CODES, SUPERVISOR_STAGE_CODES,
    StatusDecoder, StatusFrame, StatusWriteError, SupervisorProtocol, TerminalStatus,
    classify_status_write, decode_exec_error, encode_status, helper_exit_before_executed,
    managed_signals,
};

#[derive(Default)]
struct ContainmentFake {
    terminations: usize,
    reaps: usize,
    fail_termination: bool,
    fail_reap: bool,
}

impl ContainmentBackend for ContainmentFake {
    fn terminate_confirmed_group(&mut self, _group: GroupIdentity) -> Result<(), ProtocolError> {
        self.terminations += 1;
        if self.fail_termination {
            Err(ProtocolError::GroupMismatch)
        } else {
            Ok(())
        }
    }

    fn reap_direct_child(&mut self, _child: ChildIdentity) -> Result<(), ProtocolError> {
        self.reaps += 1;
        if self.fail_reap {
            Err(ProtocolError::ChildExitedEarly)
        } else {
            Ok(())
        }
    }
}

fn ready_protocol() -> SupervisorProtocol {
    let mut protocol = SupervisorProtocol::new();
    protocol
        .confirm_initial_stop(InitialStop {
            child: ChildIdentity::new(11),
            group: GroupIdentity::new(11),
            direct_parent: true,
            stop_signal: 19,
            event: 0,
        })
        .expect("initial stop");
    protocol.install_trace_options(true).expect("trace options");
    protocol.emit_ready().expect("ready");
    protocol.release_initial_stop(true).expect("continue");
    protocol
}

#[test]
fn status_decoder_accepts_fragmented_and_coalesced_frames_without_a_probe() {
    let bytes = [
        encode_status(StatusFrame::Ready),
        encode_status(StatusFrame::Executed),
        encode_status(StatusFrame::Exited(0)),
    ]
    .concat();
    assert!(bytes.len() <= 27);

    let mut coalesced = StatusDecoder::new();
    let frames = coalesced.feed(&bytes).expect("coalesced frames");
    assert_eq!(
        frames,
        [
            StatusFrame::Ready,
            StatusFrame::Executed,
            StatusFrame::Exited(0)
        ]
    );
    assert_eq!(
        coalesced.finish_eof().expect("terminal eof"),
        TerminalStatus::Exited(0)
    );

    for split in 0..=bytes.len() {
        let mut decoder = StatusDecoder::new();
        for chunk in bytes[..split].chunks(1) {
            decoder.feed(chunk).expect("fragment prefix");
        }
        for chunk in bytes[split..].chunks(1) {
            decoder.feed(chunk).expect("fragment suffix");
        }
        assert_eq!(
            decoder.finish_eof().expect("fragmented eof"),
            TerminalStatus::Exited(0)
        );
    }
}

#[test]
fn status_decoder_rejects_header_order_length_and_capacity_mutations() {
    let ready = encode_status(StatusFrame::Ready);
    let mut bad_magic = ready.clone();
    bad_magic[0] = b'X';
    assert_eq!(
        StatusDecoder::new().feed(&bad_magic),
        Err(ProtocolError::BadMagic)
    );

    let mut bad_version = ready.clone();
    bad_version[4] = 2;
    assert_eq!(
        StatusDecoder::new().feed(&bad_version),
        Err(ProtocolError::BadVersion)
    );

    let mut bad_type = ready.clone();
    bad_type[5] = 9;
    assert_eq!(
        StatusDecoder::new().feed(&bad_type),
        Err(ProtocolError::UnknownType)
    );

    let mut bad_length = ready.clone();
    bad_length[7] = 1;
    assert_eq!(
        StatusDecoder::new().feed(&bad_length),
        Err(ProtocolError::InvalidLength)
    );

    let mut bad_order = StatusDecoder::new();
    assert_eq!(
        bad_order.feed(&encode_status(StatusFrame::Executed)),
        Err(ProtocolError::OutOfOrder)
    );
    let mut duplicate = StatusDecoder::new();
    duplicate.feed(&ready).expect("ready");
    assert_eq!(duplicate.feed(&ready), Err(ProtocolError::OutOfOrder));

    let mut trailing = StatusDecoder::new();
    trailing
        .feed(
            &[
                encode_status(StatusFrame::Ready),
                encode_status(StatusFrame::Executed),
                encode_status(StatusFrame::Exited(0)),
            ]
            .concat(),
        )
        .expect("terminal");
    assert_eq!(trailing.feed(&[0]), Err(ProtocolError::TrailingFrame));

    let mut partial = StatusDecoder::new();
    partial.feed(&ready[..3]).expect("partial header");
    assert_eq!(partial.finish_eof(), Err(ProtocolError::PartialEof));
    assert_eq!(
        StatusDecoder::new().feed(&[0; 28]),
        Err(ProtocolError::BufferOverflow)
    );
}

#[test]
fn status_decoder_rejects_invalid_signal_and_eof_before_terminal() {
    let mut invalid = encode_status(StatusFrame::Signaled(1));
    invalid[8] = 65;
    let mut decoder = StatusDecoder::new();
    decoder
        .feed(&encode_status(StatusFrame::Ready))
        .expect("ready");
    decoder
        .feed(&encode_status(StatusFrame::Executed))
        .expect("executed");
    assert_eq!(
        decoder.feed(&invalid[0..]),
        Err(ProtocolError::InvalidSignal)
    );
    assert_eq!(
        StatusDecoder::new().finish_eof(),
        Err(ProtocolError::EofBeforeTerminal)
    );
}

#[test]
fn exec_error_transport_accepts_one_record_and_distinguishes_eof_cases() {
    let record = [b'D', b'2', b'B', b'E', 1, 1, 0, 7];
    assert_eq!(
        decode_exec_error(&record, true),
        Ok(Some(ExecErrorRecord { code: 7 }))
    );
    assert_eq!(
        decode_exec_error(&record[..3], true),
        Err(ProtocolError::ExecErrorPartial)
    );
    assert_eq!(
        decode_exec_error(&record, false),
        Err(ProtocolError::ExecErrorHeldOpen)
    );
    assert_eq!(
        decode_exec_error(&[], true),
        Err(ProtocolError::EmptyExecErrorEof)
    );
    let mut overlong = record.to_vec();
    overlong.push(0);
    assert_eq!(
        decode_exec_error(&overlong, true),
        Err(ProtocolError::ExecErrorOverlong)
    );
    overlong.push(0);
    assert_eq!(
        decode_exec_error(&overlong, true),
        Err(ProtocolError::ExecErrorOverlong)
    );
    let mut unknown = record;
    unknown[5] = 2;
    assert_eq!(
        decode_exec_error(&unknown, true),
        Err(ProtocolError::ExecErrorUnknown)
    );
}

#[test]
fn exact_initial_stop_options_ready_continue_exec_and_detach_order_is_required() {
    let mut protocol = SupervisorProtocol::new();
    assert_eq!(
        protocol.release_initial_stop(true),
        Err(ProtocolError::OutOfOrder)
    );
    assert_eq!(
        protocol.confirm_initial_stop(InitialStop {
            child: ChildIdentity::new(11),
            group: GroupIdentity::new(11),
            direct_parent: false,
            stop_signal: 19,
            event: 0,
        }),
        Err(ProtocolError::GroupMismatch)
    );
    assert_eq!(
        protocol.confirm_initial_stop(InitialStop {
            child: ChildIdentity::new(11),
            group: GroupIdentity::new(11),
            direct_parent: true,
            stop_signal: 18,
            event: 0,
        }),
        Err(ProtocolError::WrongInitialStop)
    );
    protocol
        .confirm_initial_stop(InitialStop {
            child: ChildIdentity::new(11),
            group: GroupIdentity::new(11),
            direct_parent: true,
            stop_signal: 19,
            event: 0,
        })
        .expect("initial stop");
    protocol.install_trace_options(true).expect("options");
    protocol.emit_ready().expect("ready");
    protocol.release_initial_stop(true).expect("continue");
    assert_eq!(
        protocol.handle_exec_stop(
            ExecStop {
                child: ChildIdentity::new(11),
                stop_signal: 5,
                event: PTRACE_EVENT_EXEC,
            },
            true,
        ),
        Ok(StatusFrame::Executed)
    );
    assert!(protocol.audit_executed());
    assert_eq!(
        protocol.handle_terminal(TerminalStatus::Exited(0)),
        Ok(StatusFrame::Exited(0))
    );
}

#[test]
fn wrong_event_wrong_child_plain_stop_and_detach_failure_never_publish_execution() {
    let mut protocol = ready_protocol();
    for stop in [
        ExecStop {
            child: ChildIdentity::new(12),
            stop_signal: 5,
            event: PTRACE_EVENT_EXEC,
        },
        ExecStop {
            child: ChildIdentity::new(11),
            stop_signal: 5,
            event: 0,
        },
        ExecStop {
            child: ChildIdentity::new(11),
            stop_signal: 19,
            event: PTRACE_EVENT_EXEC,
        },
    ] {
        assert_eq!(
            protocol.handle_exec_stop(stop, true),
            Err(ProtocolError::ExecEventMissing)
        );
        assert!(!protocol.audit_executed());
    }
    assert_eq!(
        protocol.handle_exec_stop(
            ExecStop {
                child: ChildIdentity::new(11),
                stop_signal: 5,
                event: PTRACE_EVENT_EXEC,
            },
            false,
        ),
        Err(ProtocolError::DetachFailed)
    );
    assert!(!protocol.audit_executed());
}

#[test]
fn before_exec_signal_is_coalesced_and_helper_owns_confirmed_group_cleanup() {
    let mut protocol = ready_protocol();
    let mut containment = ContainmentFake::default();
    assert_eq!(
        protocol.handle_before_exec_signal(&mut containment),
        Err(ProtocolError::PreExecTermination)
    );
    assert_eq!(containment.terminations, 1);
    assert_eq!(containment.reaps, 1);
    assert!(protocol.termination_requested());
    assert!(!protocol.audit_executed());
    assert_eq!(
        protocol.handle_before_exec_signal(&mut containment),
        Err(ProtocolError::PreExecTermination)
    );
    assert_eq!(containment.terminations, 1);
    assert_eq!(containment.reaps, 1);
    assert!(
        protocol
            .events()
            .iter()
            .all(|event| !event.audit_executed && event.frame != Some(StatusFrame::Executed))
    );
}

#[test]
fn pending_signal_before_group_confirmation_and_before_exec_death_are_fail_closed() {
    let mut protocol = SupervisorProtocol::new();
    let mut containment = ContainmentFake::default();
    assert_eq!(
        protocol.handle_before_exec_signal(&mut containment),
        Err(ProtocolError::GroupMismatch)
    );
    assert!(protocol.termination_requested());
    assert_eq!(containment.terminations, 0);

    let mut protocol = ready_protocol();
    assert_eq!(
        protocol.handle_before_exec_death(),
        Err(ProtocolError::PreExecDeath)
    );
    assert!(!protocol.audit_executed());
    assert_eq!(
        protocol.handle_terminal(TerminalStatus::Exited(0)),
        Err(ProtocolError::StatusNotExecuted)
    );
}

#[test]
fn containment_failure_preserves_the_first_cleanup_stage() {
    let mut protocol = ready_protocol();
    let mut containment = ContainmentFake {
        fail_termination: true,
        ..ContainmentFake::default()
    };
    assert_eq!(
        protocol.handle_before_exec_signal(&mut containment),
        Err(ProtocolError::GroupMismatch)
    );
    assert_eq!(containment.terminations, 1);
    assert_eq!(containment.reaps, 0);
}

#[test]
fn closed_status_reader_and_helper_crash_before_executed_are_typed_failures() {
    assert_eq!(
        classify_status_write(StatusWriteError::ClosedReader),
        Err(ProtocolError::StatusEpipe)
    );
    assert_eq!(
        helper_exit_before_executed(),
        Err(ProtocolError::HelperBeforeExecuted)
    );
}

#[test]
fn parent_supervisor_and_child_stage_tables_are_closed_and_nonempty() {
    for table in [
        RUST_PARENT_STAGE_CODES,
        SUPERVISOR_STAGE_CODES,
        CHILD_STAGE_CODES,
    ] {
        assert!(!table.is_empty());
        assert!(table.iter().all(|code| code.starts_with("D2B-BZLEXEC-")));
        for (index, code) in table.iter().enumerate() {
            assert!(
                !table[index + 1..].contains(code),
                "duplicate stage code {code}"
            );
        }
    }
    assert!(SUPERVISOR_STAGE_CODES.contains(&"D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION"));
    assert!(CHILD_STAGE_CODES.contains(&"D2B-BZLEXEC-CHILD-EXECVEAT"));
}

struct RealBinaries {
    supervisor: PathBuf,
    plant: PathBuf,
    decoder: PathBuf,
}

static REAL_BINARIES: OnceLock<RealBinaries> = OnceLock::new();

fn real_binaries() -> &'static RealBinaries {
    REAL_BINARIES.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repository root");
        let source_dir = root.join("tests/tools/d2b-bazel-exec-supervisor");
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let output_dir =
            std::env::temp_dir().join(format!("d2b-bazel-exec-supervisor-tests-{suffix}"));
        fs::create_dir(&output_dir).expect("test output directory");
        let supervisor = output_dir.join("supervisor");
        let plant = output_dir.join("plant");
        let decoder = output_dir.join("decoder");
        let cc = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
        for (source, output) in [
            (source_dir.join("supervisor.c"), supervisor.clone()),
            (source_dir.join("sandbox-crash-plant.c"), plant.clone()),
        ] {
            let result = Command::new(&cc)
                .args([
                    "-std=c11",
                    "-O2",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-Wno-unused-parameter",
                    "-fno-pie",
                    "-no-pie",
                ])
                .arg(source)
                .arg("-o")
                .arg(&output)
                .status()
                .expect("C compiler for real supervisor test");
            assert!(result.success(), "real C test binary compilation failed");
        }

        let source = source_dir
            .join("supervisor.c")
            .to_str()
            .expect("UTF-8 supervisor source")
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let harness = real_decoder_harness(&source);
        let harness_path = output_dir.join("decoder.c");
        fs::write(&harness_path, harness).expect("decoder harness");
        let result = Command::new(&cc)
            .args([
                "-std=c11",
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Wno-unused-parameter",
                "-fno-pie",
                "-no-pie",
            ])
            .arg(&harness_path)
            .arg("-o")
            .arg(&decoder)
            .status()
            .expect("C compiler for exec transport test");
        assert!(
            result.success(),
            "exec transport harness compilation failed"
        );

        RealBinaries {
            supervisor,
            plant,
            decoder,
        }
    })
}

fn real_decoder_harness(supervisor_source: &str) -> String {
    r#"
#define main d2b_embedded_supervisor_main
#include "__SUPERVISOR_SOURCE__"
#undef main

#include <signal.h>
#include <sys/time.h>

static void write_all(int fd, const unsigned char *bytes, size_t length) {
  size_t offset = 0;
  while (offset < length) {
    ssize_t result = write(fd, bytes + offset, length - offset);
    if (result > 0) {
      offset += (size_t)result;
    } else if (result < 0 && errno == EINTR) {
      continue;
    } else {
      _exit(91);
    }
  }
}

static void interrupt_without_restart(int signal_number) {
  (void)signal_number;
}

static int decode_mode(const char *mode, unsigned char child_code) {
  int pipe_flags = strcmp(mode, "eintr") == 0 ? 0 : O_NONBLOCK;
  int descriptors[2];
  if (pipe2(descriptors, pipe_flags | O_CLOEXEC) != 0) return 90;

  pid_t writer = fork();
  if (writer < 0) return 89;
  if (writer == 0) {
    close(descriptors[0]);
    unsigned char record[D2B_EXEC_ERROR_SIZE] =
        {'D', '2', 'B', 'E', D2B_PROTOCOL_VERSION, 1, 0, child_code};
    if (strcmp(mode, "partial") == 0) {
      write_all(descriptors[1], record, 3);
    } else if (strcmp(mode, "fragmented") == 0) {
      write_all(descriptors[1], record, 3);
      usleep(20000);
      write_all(descriptors[1], record + 3, 5);
    } else if (strcmp(mode, "overlong") == 0) {
      write_all(descriptors[1], record, sizeof(record));
      write_all(descriptors[1], (const unsigned char *)"x", 1);
    } else {
      if (strcmp(mode, "eagain") == 0) usleep(20000);
      write_all(descriptors[1], record, sizeof(record));
    }
    if (strcmp(mode, "held-open") == 0) {
      usleep(1000000);
    } else {
      close(descriptors[1]);
    }
    _exit(0);
  }
  close(descriptors[1]);

  if (strcmp(mode, "eintr") == 0) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = interrupt_without_restart;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGALRM, &action, NULL) != 0) return 88;
    ualarm(10000, 0);
  }

  unsigned char record[D2B_EXEC_ERROR_SIZE] = {0};
  size_t length = 0;
  int eof = 0;
  int probe_complete = 0;
  int pending = 0;
  int64_t deadline = d2b_monotonic_ms() + 300;
  int result = D2B_EXEC_RECORD_PENDING;
  while (d2b_remaining_ms(deadline) > 0) {
    result = d2b_read_exec_record(descriptors[0], record, &length, &eof,
                                  &probe_complete, deadline);
    if (result != D2B_EXEC_RECORD_PENDING) break;
    pending = 1;
    struct pollfd descriptor = {.fd = descriptors[0], .events = POLLIN | POLLHUP};
    int poll_result = poll(&descriptor, 1, d2b_remaining_ms(deadline));
    if (poll_result < 0 && errno == EINTR) continue;
    if (poll_result <= 0) {
      result = D2B_EXEC_RECORD_TIMEOUT;
      break;
    }
  }
  if (strcmp(mode, "eintr") == 0) ualarm(0, 0);
  if (result == D2B_EXEC_RECORD_PENDING) result = D2B_EXEC_RECORD_TIMEOUT;
  close(descriptors[0]);
  if (result == D2B_EXEC_RECORD_TIMEOUT && strcmp(mode, "held-open") == 0) {
    kill(writer, SIGKILL);
  }
  waitpid(writer, NULL, 0);
  const char *stage = "none";
  if (result == D2B_EXEC_RECORD_COMPLETE) stage = d2b_child_error_code(record[7]);
  printf("%d %d %s\n", result, pending, stage == NULL ? "none" : stage);
  return 0;
}

int main(int argc, char **argv) {
  if (argc < 2) return 87;
  unsigned char child_code = argc > 2 ? (unsigned char)strtoul(argv[2], NULL, 10)
                                      : D2B_CHILD_EXECVEAT;
  return decode_mode(argv[1], child_code);
}
"#
    .replace("__SUPERVISOR_SOURCE__", supervisor_source)
}

fn decoder_observation(mode: &str, child_code: u8) -> (i32, bool, String) {
    let output = Command::new(&real_binaries().decoder)
        .arg(mode)
        .arg(child_code.to_string())
        .output()
        .expect("exec transport harness");
    assert!(
        output.status.success(),
        "exec transport harness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fields = String::from_utf8(output.stdout)
        .expect("decoder output")
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 3, "decoder output fields: {fields:?}");
    (
        fields[0].parse().expect("decoder result"),
        fields[1].parse::<u8>().expect("pending flag") != 0,
        fields[2].clone(),
    )
}

enum StatusEvent {
    Executed,
    Complete(Vec<u8>),
}

fn status_events(mut reader: File) -> Receiver<StatusEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut announced = false;
        let mut buffer = [0_u8; 64];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    bytes.extend_from_slice(&buffer[..length]);
                    if !announced && bytes.len() >= 16 {
                        announced = true;
                        if sender.send(StatusEvent::Executed).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => return,
            }
        }
        let _ = sender.send(StatusEvent::Complete(bytes));
    });
    receiver
}

fn spawn_real_supervisor(target: &Path, arguments: &[&str]) -> (Child, File) {
    let (status_reader, status_writer) = pipe2(OFlag::O_CLOEXEC).expect("status transport pipe");
    let target_fd: OwnedFd = File::open(target).expect("target executable").into();
    let mut command = Command::new(&real_binaries().supervisor);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
        .fd_mappings(vec![
            FdMapping {
                parent_fd: status_writer,
                child_fd: 8,
            },
            FdMapping {
                parent_fd: target_fd,
                child_fd: 9,
            },
        ])
        .expect("status and executable fd mappings");

    let previous_mask = SigSet::thread_get_mask().expect("test signal mask");
    managed_signals()
        .thread_block()
        .expect("block managed signals for helper handoff");
    let child = command.spawn();
    previous_mask
        .thread_set_mask()
        .expect("restore test signal mask");
    (
        child.expect("spawn real C supervisor"),
        File::from(status_reader),
    )
}

fn wait_for_complete(receiver: &Receiver<StatusEvent>, child: &mut Child) -> Vec<u8> {
    loop {
        let event = match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(event) => event,
            Err(error) => {
                let _ = signal::kill(
                    nix::unistd::Pid::from_raw(child.id() as i32),
                    signal::Signal::SIGKILL,
                );
                let _ = child.wait();
                panic!("real supervisor status deadline: {error}");
            }
        };
        match event {
            StatusEvent::Executed => {}
            StatusEvent::Complete(bytes) => return bytes,
        }
        if child.try_wait().expect("supervisor wait probe").is_some() {
            continue;
        }
    }
}

#[test]
fn real_c_exec_error_transport_covers_deadline_and_closed_mapping() {
    for code in 1_u8..=8 {
        let (result, _, stage) = decoder_observation("exact", code);
        assert_eq!(result, 2, "complete code {code}");
        assert_eq!(stage, CHILD_STAGE_CODES[usize::from(code - 1)]);
    }

    let (result, _, _) = decoder_observation("overlong", 8);
    assert_eq!(result, -2);
    let (result, _, _) = decoder_observation("partial", 8);
    assert_eq!(result, -1);
    let (result, _, _) = decoder_observation("unknown", 99);
    assert_eq!(result, -3);
    let (result, pending, _) = decoder_observation("held-open", 8);
    assert_eq!(result, -5);
    assert!(pending);

    let (result, pending, stage) = decoder_observation("eintr", 8);
    assert_eq!(result, 2);
    assert!(!pending);
    assert_eq!(stage, CHILD_STAGE_CODES[7]);
    let (result, pending, stage) = decoder_observation("eagain", 8);
    assert_eq!(result, 2);
    assert!(pending);
    assert_eq!(stage, CHILD_STAGE_CODES[7]);
    let (result, pending, stage) = decoder_observation("fragmented", 8);
    assert_eq!(result, 2);
    assert!(pending);
    assert_eq!(stage, CHILD_STAGE_CODES[7]);
}

#[test]
fn real_c_supervisor_closes_signalfd_and_preserves_order_on_fast_exit() {
    let binaries = real_binaries();
    let (mut child, reader) =
        spawn_real_supervisor(&binaries.plant, &["plant", "--stage", "fd-audit"]);
    let receiver = status_events(reader);
    let status = wait_for_complete(&receiver, &mut child);
    let exit = child.wait().expect("fd-audit supervisor wait");
    assert_eq!(exit.code(), Some(0));
    assert_eq!(
        status,
        [
            encode_status(StatusFrame::Ready),
            encode_status(StatusFrame::Executed),
            encode_status(StatusFrame::Exited(0)),
        ]
        .concat()
    );
}

#[test]
fn real_c_supervisor_maps_child_exec_failure_without_false_executed_success() {
    let (mut child, reader) = spawn_real_supervisor(Path::new("/dev/null"), &["target"]);
    let receiver = status_events(reader);
    let status = wait_for_complete(&receiver, &mut child);
    let mut stderr = child.stderr.take().expect("supervisor stderr");
    let exit = child.wait().expect("exec-error supervisor wait");
    let mut diagnostics = String::new();
    stderr
        .read_to_string(&mut diagnostics)
        .expect("exec-error diagnostics");
    assert_eq!(exit.code(), Some(1));
    assert_eq!(status, encode_status(StatusFrame::Ready));
    assert!(diagnostics.contains("D2B-BZLEXEC-CHILD-EXECVEAT"));
    assert!(!diagnostics.contains("D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH"));
}

#[test]
fn real_c_supervisor_reaps_once_after_full_grace_when_leader_exits_early() {
    let binaries = real_binaries();
    let (mut child, reader) =
        spawn_real_supervisor(&binaries.plant, &["plant", "--stage", "exit-during-grace"]);
    let receiver = status_events(reader);
    match receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("EXECUTED status deadline")
    {
        StatusEvent::Executed => {}
        StatusEvent::Complete(_) => panic!("terminal status preceded EXECUTED"),
    }
    let started = Instant::now();
    signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        signal::Signal::SIGTERM,
    )
    .expect("forward termination");
    let status = wait_for_complete(&receiver, &mut child);
    let elapsed = started.elapsed();
    let exit = child.wait().expect("grace supervisor wait");
    assert_eq!(exit.code(), Some(143));
    assert!(
        elapsed >= Duration::from_millis(900),
        "termination grace was shortened: {elapsed:?}"
    );
    assert_eq!(
        status,
        [
            encode_status(StatusFrame::Ready),
            encode_status(StatusFrame::Executed),
            encode_status(StatusFrame::Signaled(15)),
        ]
        .concat()
    );
}

#[test]
fn real_c_supervisor_classifies_closed_status_reader_as_epipe_without_success() {
    let binaries = real_binaries();
    let (mut child, reader) =
        spawn_real_supervisor(&binaries.plant, &["plant", "--stage", "after-executed"]);
    drop(reader);
    let mut stderr = child.stderr.take().expect("supervisor stderr");
    let exit = child.wait().expect("EPIPE supervisor wait");
    let mut diagnostics = String::new();
    stderr
        .read_to_string(&mut diagnostics)
        .expect("EPIPE diagnostics");
    assert_eq!(exit.code(), Some(1));
    assert!(diagnostics.contains("D2B-BZLEXEC-HELPER-EXEC-EPIPE"));
}
