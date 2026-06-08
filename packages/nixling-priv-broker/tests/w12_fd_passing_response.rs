//! Integration: send_json_frame_with_fds delivers JSON body + SCM_RIGHTS
//! attachment in a single seqpacket frame.
//!
//! This exercises the new `protocol::send_json_frame_with_fds`
//! helper that the broker's `handle_connection` uses to return the
//! `OpenPidfd` / `SpawnRunner` pidfd alongside the JSON response
//! body. Validates:
//!
//! 1. The JSON body deserialises on the receiver into a
//!    representative response struct.
//! 2. Exactly one fd is received over SCM_RIGHTS.
//! 3. The received fd is a working pipe-read end (proves it's a
//!    duplicated kernel handle, not the sender's own fd).
//! 4. With an empty fd slice, the helper degrades to a byte-
//!    equivalent `send()` path that downstream callers can still
//!    consume via `recv_json_frame`.

use std::os::fd::AsRawFd;

use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
use nix::unistd::{close, pipe, read, write};
use nixling_priv_broker::fd_passing::recv_fds;
use nixling_priv_broker::protocol::{recv_json_frame, send_json_frame, send_json_frame_with_fds};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Reply {
    kind: String,
    pid: i32,
}

#[test]
fn send_json_frame_with_fds_delivers_body_and_pidfd_shaped_attachment() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");
    let (read_end, write_end) = pipe().expect("pipe");

    let body = Reply {
        kind: "OpenPidfd".into(),
        pid: 4242,
    };
    send_json_frame_with_fds(left.as_raw_fd(), &body, &[read_end.as_raw_fd()])
        .expect("send body + fd");

    let (payload, fds) = recv_fds(right.as_raw_fd()).expect("recv body + fds");
    assert_eq!(fds.len(), 1);

    // The framing prefix (4-byte LE u32) precedes the JSON body.
    assert!(payload.len() >= 4);
    let declared = u32::from_le_bytes(payload[..4].try_into().expect("prefix len")) as usize;
    assert_eq!(declared, payload.len() - 4);
    let parsed: Reply = serde_json::from_slice(&payload[4..]).expect("decode body");
    assert_eq!(parsed, body);

    // The received fd really is a duplicated handle: writing to the
    // original `write_end` is observable through the SCM_RIGHTS copy.
    write(&write_end, b"ok").expect("write to original");
    let mut buf = [0_u8; 2];
    read(fds[0], &mut buf).expect("read through scm_rights copy");
    assert_eq!(&buf, b"ok");
    close(fds[0]).expect("close received fd");
}

#[test]
fn send_json_frame_with_no_fds_is_byte_equivalent_to_send_json_frame() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");

    let body = Reply {
        kind: "Ack".into(),
        pid: 7,
    };
    send_json_frame_with_fds(left.as_raw_fd(), &body, &[]).expect("send body");

    let decoded = recv_json_frame::<Reply>(right.as_raw_fd())
        .expect("recv frame")
        .expect("frame present");
    assert_eq!(decoded, body);
}

#[test]
fn send_json_frame_helper_still_works_after_w12_refactor() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");
    let body = Reply {
        kind: "Hello".into(),
        pid: 1,
    };
    send_json_frame(left.as_raw_fd(), &body).expect("send body");
    let decoded = recv_json_frame::<Reply>(right.as_raw_fd())
        .expect("recv frame")
        .expect("frame present");
    assert_eq!(decoded, body);
}
