//! Layer-1 transport coverage for the typed Zone store handoff.

use std::os::fd::AsRawFd;
use std::process::Command;

use d2b_contracts::broker_wire::{BrokerResponse, OpenZoneStoreResponse, ZoneStoreDisposition};
use d2b_contracts::v3::storage::ZoneStoreId;
use d2b_priv_broker::fd_passing::{FdPassingError, recv_one_fd, send_fds};
use d2b_priv_broker::ops::audit_op::OperationFields;
use d2b_priv_broker::protocol::send_json_frame_with_fds;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
use nix::unistd::{close, pipe};

fn cloexec_set(fd: i32) -> bool {
    fcntl(fd, FcntlArg::F_GETFD)
        .map(|flags| FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC))
        .unwrap_or(false)
}

#[test]
fn open_zone_store_response_transfers_exactly_one_cloexec_fd() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");
    let (read_end, _write_end) = pipe().expect("pipe");
    let response = BrokerResponse::OpenZoneStore(OpenZoneStoreResponse {
        zone_store_id: ZoneStoreId::parse("zone-store-local-root").expect("id"),
        store_identity: format!("sha256:{}", "b".repeat(64)),
        disposition: ZoneStoreDisposition::Opened,
        fd_index: 0,
    });

    send_json_frame_with_fds(left.as_raw_fd(), &response, &[read_end.as_raw_fd()])
        .expect("send response and one fd");
    let (frame, received_fd) = recv_one_fd(right.as_raw_fd()).expect("receive one fd");
    assert!(cloexec_set(received_fd), "MSG_CMSG_CLOEXEC was not applied");
    let declared = u32::from_le_bytes(frame[..4].try_into().expect("length prefix")) as usize;
    assert_eq!(declared, frame.len() - 4);
    let decoded: BrokerResponse = serde_json::from_slice(&frame[4..]).expect("decode response");
    assert_eq!(decoded, response);

    let mut child = Command::new("sleep")
        .arg("2")
        .spawn()
        .expect("fork and exec probe");
    let child_fd = format!("/proc/{}/fd/{received_fd}", child.id());
    let child_comm = format!("/proc/{}/comm", child.id());
    let mut inherited = true;
    let mut observed_exec = false;
    for _ in 0..50 {
        if let Ok(comm) = std::fs::read_to_string(&child_comm)
            && comm.trim() == "sleep"
        {
            observed_exec = true;
            inherited = std::path::Path::new(&child_fd).exists();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    close(received_fd).expect("close received fd");
    assert!(observed_exec, "exec probe did not reach sleep");
    assert!(!inherited, "received store fd inherited across exec");
}

#[test]
fn open_zone_store_handoff_rejects_more_than_one_fd() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");
    let (first, _first_write) = pipe().expect("first pipe");
    let (second, _second_write) = pipe().expect("second pipe");
    send_fds(
        left.as_raw_fd(),
        b"zone-store",
        &[first.as_raw_fd(), second.as_raw_fd()],
    )
    .expect("send two fds");
    assert_eq!(
        recv_one_fd(right.as_raw_fd()).expect_err("two fds must be refused"),
        FdPassingError::UnexpectedFdCount {
            expected: 1,
            actual: 2
        }
    );
}

#[test]
fn open_zone_store_audit_fields_are_redacted() {
    let fields = OperationFields::OpenZoneStore {
        zone_store_id: "zone-store-local-root".to_owned(),
        store_identity: format!("sha256:{}", "c".repeat(64)),
        disposition: "opened".to_owned(),
        fd_count: 1,
    };
    let encoded = serde_json::to_string(&fields).expect("serialize audit fields");
    assert!(encoded.contains("OpenZoneStore") || encoded.contains("zone-store-local-root"));
    assert!(!encoded.contains("store.redb"));
    assert!(!encoded.contains("storage.json"));
    assert!(!encoded.contains("/var/"));
    assert!(!encoded.contains("parentDirectory"));
    let parsed = OperationFields::from_operation_value(
        "OpenZoneStore",
        serde_json::from_str(&encoded).expect("parse audit JSON"),
    )
    .expect("parse OpenZoneStore audit fields");
    assert_eq!(parsed, fields);
}
