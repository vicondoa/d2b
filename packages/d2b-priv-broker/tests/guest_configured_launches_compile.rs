use d2b_contracts::{
    v2_guest_configured_launches::{GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesV1},
    v2_identity::{RealmId, WorkloadId},
};
use d2b_core::configured_argv::ConfiguredArgv;
use d2b_realm_core::ProtocolToken;
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
fn broker_can_encode_one_filtered_workload_catalog() {
    let item = GuestConfiguredLaunchEntryV1::new(
        ProtocolToken::parse("browser").unwrap(),
        ConfiguredArgv::new(vec!["firefox".to_owned()]).unwrap(),
        true,
    )
    .unwrap();
    let catalog = GuestConfiguredLaunchesV1::new(
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        [0x44; 32],
        vec![item],
    )
    .unwrap();
    let encoded = catalog.encode().unwrap();
    let mut writer = CountingWriter(0);
    encoded.write_to(&mut writer).unwrap();
    assert_eq!(writer.0, encoded.as_slice().len());
    assert_ne!(encoded.sha256(), [0; 32]);
}
