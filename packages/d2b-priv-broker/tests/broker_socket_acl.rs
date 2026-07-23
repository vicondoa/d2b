#![cfg(feature = "layer1-bootstrap")]

mod common;

use common::TestBroker;

#[test]
fn socket_acl_rejects_non_d2bd_peers_and_accepts_daemon() {
    let broker = TestBroker::spawn("broker-socket-acl-");
    assert_eq!(broker.socket_mode(), 0o660, "expected socket mode 660");

    let launcher_uid = 2001;
    let admin_uid = 2002;
    for (label, denied_uid) in [
        ("root", 0),
        ("launcher", launcher_uid),
        ("admin", admin_uid),
    ] {
        let output = broker.probe_hello(denied_uid);
        output.assert_success();
        assert!(
            output
                .stdout()
                .contains("\"kind\":\"Broker.PeerCredentialRefused\""),
            "{label} peer stdout did not report typed peer refusal: {}",
            output.stdout()
        );
    }

    let hello = broker.probe_hello(broker.d2bd_uid());
    hello.assert_success();
    assert!(
        hello.stdout().contains("\"response\":\"HelloOk\""),
        "d2bd peer did not receive HelloOk: {}",
        hello.stdout()
    );

    let audit = broker.audit_contents();
    for denied_uid in [0, launcher_uid, admin_uid] {
        assert!(
            audit.contains(&format!("\"caller_uid\":{denied_uid}")),
            "missing denied audit row for uid {denied_uid}:\n{audit}"
        );
    }
}
