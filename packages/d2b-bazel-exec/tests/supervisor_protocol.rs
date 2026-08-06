use d2b_bazel_exec::{
    CHILD_STAGE_CODES, ChildIdentity, ContainmentBackend, ExecErrorRecord, ExecStop, GroupIdentity,
    InitialStop, PTRACE_EVENT_EXEC, ProtocolError, RUST_PARENT_STAGE_CODES, SUPERVISOR_STAGE_CODES,
    StatusDecoder, StatusFrame, StatusWriteError, SupervisorProtocol, TerminalStatus,
    classify_status_write, decode_exec_error, encode_status, helper_exit_before_executed,
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
