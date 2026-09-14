use d2b_provider_guest_qemu_media::{
    QmpCommand, QmpError, QmpGreeting, QmpReply, QmpSession, QmpTransport, ScriptedQmpTransport,
};
use std::collections::VecDeque;

#[test]
fn negotiation_then_media_hotplug_uses_ordered_qmp_commands() {
    let transport = ScriptedQmpTransport::new()
        .with_greeting("8.2")
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok());
    let mut session = QmpSession::new(transport);
    session.negotiate().unwrap();
    session.attach_media("media-0", 3, true).unwrap();
    session.detach_media("media-0").unwrap();
    assert_eq!(
        session.commands().cloned().collect::<Vec<_>>(),
        vec![
            QmpCommand::Capabilities,
            QmpCommand::BlockdevAdd {
                node_name: "media-0".to_owned(),
                fd_slot: 3,
                read_only: true,
            },
            QmpCommand::DeviceAdd {
                device_id: "media-0".to_owned(),
                drive: "media-0".to_owned(),
            },
            QmpCommand::DeviceDel {
                device_id: "media-0".to_owned(),
            },
            QmpCommand::BlockdevDel {
                node_name: "media-0".to_owned(),
            },
        ]
    );
}

#[test]
fn failed_device_add_rolls_the_block_node_back() {
    let transport = ScriptedQmpTransport::new()
        .with_greeting("8.2")
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok())
        .with_error(QmpError::CommandFailed)
        .with_reply(QmpReply::ok());
    let mut session = QmpSession::new(transport);
    session.negotiate().unwrap();
    assert_eq!(
        session.attach_media("media-0", 3, true).unwrap_err(),
        QmpError::CommandFailed
    );
    assert_eq!(
        session.commands().cloned().collect::<Vec<_>>(),
        vec![
            QmpCommand::Capabilities,
            QmpCommand::BlockdevAdd {
                node_name: "media-0".to_owned(),
                fd_slot: 3,
                read_only: true,
            },
            QmpCommand::DeviceAdd {
                device_id: "media-0".to_owned(),
                drive: "media-0".to_owned(),
            },
            QmpCommand::BlockdevDel {
                node_name: "media-0".to_owned(),
            },
        ]
    );
}

#[test]
fn unnegotiated_session_rejects_media_commands() {
    let transport = ScriptedQmpTransport::new()
        .with_greeting("8.2")
        .with_error(QmpError::CapabilitiesFailed);
    let mut session = QmpSession::new(transport);
    assert!(session.negotiate().is_err());
    assert_eq!(
        session.attach_media("media-0", 3, true).unwrap_err(),
        QmpError::NotReady
    );
    assert_eq!(session.commands().count(), 1);
}

#[derive(Default)]
struct RenegotiatingTransport {
    greetings: VecDeque<QmpGreeting>,
    replies: VecDeque<Result<QmpReply, QmpError>>,
}

impl QmpTransport for RenegotiatingTransport {
    fn receive_greeting(&mut self) -> Result<QmpGreeting, QmpError> {
        self.greetings.pop_front().ok_or(QmpError::GreetingTimeout)
    }

    fn execute(&mut self, _command: &QmpCommand) -> Result<QmpReply, QmpError> {
        self.replies.pop_front().unwrap_or(Err(QmpError::Timeout))
    }
}

#[test]
fn failed_renegotiation_clears_previous_session() {
    let mut transport = RenegotiatingTransport::default();
    transport.greetings.push_back(QmpGreeting {
        version: "8.2".to_owned(),
    });
    transport.greetings.push_back(QmpGreeting {
        version: "8.2".to_owned(),
    });
    transport.replies.push_back(Ok(QmpReply::ok()));
    transport
        .replies
        .push_back(Err(QmpError::CapabilitiesFailed));

    let mut session = QmpSession::new(transport);
    session.negotiate().unwrap();
    assert!(session.negotiate().is_err());
    assert_eq!(
        session.attach_media("media-0", 3, true).unwrap_err(),
        QmpError::NotReady
    );
}
