use d2b_provider_runtime_qemu_media::{
    QmpCommand, QmpHealth, QmpReply, QmpSession, ScriptedQmpTransport,
};

#[test]
fn capability_negotiation_and_boot_commands_are_typed() {
    let transport = ScriptedQmpTransport::new()
        .with_greeting("8.2")
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok())
        .with_reply(QmpReply::ok());
    let mut session = QmpSession::new(transport);
    session.negotiate().unwrap();
    session.cont().unwrap();
    assert_eq!(
        session.commands(),
        &[QmpCommand::Capabilities, QmpCommand::Cont]
    );
    assert_eq!(session.health().phase(), "ready");
}

#[test]
fn hotplug_attach_and_detach_use_ordered_qmp_commands() {
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
        session.commands(),
        &[
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
fn health_degrades_after_bounded_failures() {
    let mut health = QmpHealth::new(2);
    assert!(health.record_failure().is_ok());
    assert_eq!(health.phase(), "ready");
    assert!(health.record_failure().is_err());
    assert_eq!(health.phase(), "degraded");
}
