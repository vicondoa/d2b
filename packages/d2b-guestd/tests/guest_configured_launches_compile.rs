use d2b_contracts::v2_guest_configured_launches::{
    GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES, GUEST_CONFIGURED_LAUNCHES_MAGIC,
    GuestConfiguredLaunchesV1,
};

fn encoded_catalog() -> Vec<u8> {
    let entry_bytes = 8 + 7 + 2 + 7;
    let total_bytes = GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES + 4 + entry_bytes;
    let mut encoded = Vec::with_capacity(total_bytes);
    encoded.extend_from_slice(&GUEST_CONFIGURED_LAUNCHES_MAGIC);
    encoded.extend_from_slice(&1_u16.to_be_bytes());
    encoded.extend_from_slice(&1_u16.to_be_bytes());
    encoded.extend_from_slice(&0_u16.to_be_bytes());
    encoded.extend_from_slice(&0_u16.to_be_bytes());
    encoded.extend_from_slice(&u32::try_from(total_bytes).unwrap().to_be_bytes());
    encoded.extend_from_slice(b"aaaaaaaaaaaaaaaaaaaa");
    encoded.extend_from_slice(b"bbbbbbbbbbbbbbbbbbba");
    encoded.extend_from_slice(&[0x44; 32]);
    encoded.extend_from_slice(&1_u16.to_be_bytes());
    encoded.extend_from_slice(&0_u16.to_be_bytes());
    encoded.extend_from_slice(&u32::try_from(entry_bytes).unwrap().to_be_bytes());
    encoded.extend_from_slice(&7_u16.to_be_bytes());
    encoded.extend_from_slice(b"browser");
    encoded.extend_from_slice(&1_u16.to_be_bytes());
    encoded.extend_from_slice(&1_u16.to_be_bytes());
    encoded.extend_from_slice(&0_u16.to_be_bytes());
    encoded.extend_from_slice(&7_u16.to_be_bytes());
    encoded.extend_from_slice(b"firefox");
    encoded
}

#[test]
fn guestd_can_decode_and_resolve_the_shared_catalog() {
    let decoded = GuestConfiguredLaunchesV1::decode(&encoded_catalog()).unwrap();
    let resolved = decoded.resolve_id("browser").unwrap();
    assert_eq!(resolved.argv().as_slice(), &["firefox".to_owned()]);
    assert!(resolved.graphical());
}
