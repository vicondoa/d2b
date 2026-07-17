use d2b_contracts::v2_component_session::{
    BootstrapPskBinding, GUEST_SESSION_CREDENTIAL_V1_WITH_BOOTSTRAP_BYTES,
    GuestBootstrapCredentialV1, GuestBootstrapPsk, GuestSessionCredentialV1, OperationId,
};
use std::io::{self, Write};

struct CountingWriter(usize);

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn broker_can_encode_the_shared_guest_session_credential() {
    let psk = GuestBootstrapPsk::generate_with(|bytes| {
        bytes.fill(0x88);
        Ok(())
    })
    .unwrap();
    let bootstrap = GuestBootstrapCredentialV1::new(
        BootstrapPskBinding {
            operation_id: OperationId::new(vec![0x66; 16]).unwrap(),
            replay_nonce: [0x77; 32],
            expires_at_unix_ms: 9_000,
        },
        1_000,
        psk,
    )
    .unwrap();
    let credential = GuestSessionCredentialV1::new(
        7,
        [0x11; 32],
        [0x22; 32],
        [0x33; 32],
        [0x44; 32],
        Some(bootstrap),
    )
    .unwrap();
    let encoded = credential.encode().unwrap();
    let mut writer = CountingWriter(0);
    encoded.write_to(&mut writer).unwrap();
    assert_eq!(writer.0, GUEST_SESSION_CREDENTIAL_V1_WITH_BOOTSTRAP_BYTES);
    assert_eq!(
        format!("{encoded:?}"),
        "GuestSessionCredentialBytes(REDACTED)"
    );
}
