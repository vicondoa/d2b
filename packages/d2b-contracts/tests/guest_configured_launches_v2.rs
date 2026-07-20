#![cfg(feature = "v2-guest-configured-launches")]

use d2b_contracts::{
    v2_guest_configured_launches::*,
    v2_identity::{RealmId, WorkloadId},
};
use d2b_core::configured_argv::ConfiguredArgv;
use d2b_realm_core::ProtocolToken;
use sha2::{Digest, Sha256};

fn entry(id: &str, argv: &[&str], graphical: bool) -> GuestConfiguredLaunchEntryV1 {
    GuestConfiguredLaunchEntryV1::new(
        ProtocolToken::parse(id).unwrap(),
        ConfiguredArgv::new(argv.iter().map(|value| (*value).to_owned()).collect()).unwrap(),
        graphical,
    )
    .unwrap()
}

fn catalog() -> GuestConfiguredLaunchesV1 {
    GuestConfiguredLaunchesV1::new(
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        [0x44; 32],
        vec![
            entry("browser", &["firefox", "--private-window"], true),
            entry("console", &["foot"], false),
        ],
    )
    .unwrap()
}

fn first_entry_offsets(bytes: &[u8]) -> (usize, usize, usize, usize) {
    let entry_start = GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES;
    let id_len = usize::from(u16::from_be_bytes(
        bytes[entry_start + 4..entry_start + 6].try_into().unwrap(),
    ));
    let flags = entry_start + 6 + id_len;
    let argc = flags + 2;
    let reserved = argc + 2;
    (entry_start, flags, argc, reserved)
}

#[test]
fn configured_launches_have_a_canonical_vector_round_trip_and_digest() {
    let value = catalog();
    let encoded = value.encode().unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(&GUEST_CONFIGURED_LAUNCHES_MAGIC);
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&0_u16.to_be_bytes());
    expected.extend_from_slice(&0_u16.to_be_bytes());

    let first_entry_len = 8 + 7 + (2 + 7) + (2 + 16);
    let second_entry_len = 8 + 7 + (2 + 4);
    let expected_len =
        GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES + 4 + first_entry_len + 4 + second_entry_len;
    expected.extend_from_slice(&u32::try_from(expected_len).unwrap().to_be_bytes());
    expected.extend_from_slice(b"aaaaaaaaaaaaaaaaaaaa");
    expected.extend_from_slice(b"bbbbbbbbbbbbbbbbbbba");
    expected.extend_from_slice(&[0x44; 32]);
    expected.extend_from_slice(&2_u16.to_be_bytes());
    expected.extend_from_slice(&0_u16.to_be_bytes());

    expected.extend_from_slice(&u32::try_from(first_entry_len).unwrap().to_be_bytes());
    expected.extend_from_slice(&7_u16.to_be_bytes());
    expected.extend_from_slice(b"browser");
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&2_u16.to_be_bytes());
    expected.extend_from_slice(&0_u16.to_be_bytes());
    expected.extend_from_slice(&7_u16.to_be_bytes());
    expected.extend_from_slice(b"firefox");
    expected.extend_from_slice(&16_u16.to_be_bytes());
    expected.extend_from_slice(b"--private-window");

    expected.extend_from_slice(&u32::try_from(second_entry_len).unwrap().to_be_bytes());
    expected.extend_from_slice(&7_u16.to_be_bytes());
    expected.extend_from_slice(b"console");
    expected.extend_from_slice(&0_u16.to_be_bytes());
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&0_u16.to_be_bytes());
    expected.extend_from_slice(&4_u16.to_be_bytes());
    expected.extend_from_slice(b"foot");

    assert_eq!(encoded.as_slice(), expected.as_slice());
    assert_eq!(encoded.sha256(), Sha256::digest(&expected).as_slice());

    let decoded = GuestConfiguredLaunchesV1::decode(encoded.as_slice()).unwrap();
    assert_eq!(decoded.schema_version(), 1);
    assert_eq!(decoded.codec_version(), 1);
    assert_eq!(decoded.realm_id().as_str(), "aaaaaaaaaaaaaaaaaaaa");
    assert_eq!(decoded.workload_id().as_str(), "bbbbbbbbbbbbbbbbbbba");
    assert_eq!(decoded.workload_digest(), &[0x44; 32]);
    assert_eq!(decoded.entries().len(), 2);
    let browser = decoded
        .resolve(&ProtocolToken::parse("browser").unwrap())
        .unwrap();
    assert!(browser.graphical());
    assert_eq!(
        browser.argv().as_slice(),
        &["firefox".to_owned(), "--private-window".to_owned()]
    );
    assert!(
        decoded
            .resolve(&ProtocolToken::parse("missing").unwrap())
            .is_none()
    );
}

#[test]
fn configured_launches_reject_every_truncation_and_trailing_bytes() {
    let encoded = catalog().encode().unwrap();
    for length in 0..encoded.as_slice().len() {
        assert!(
            GuestConfiguredLaunchesV1::decode(&encoded.as_slice()[..length]).is_err(),
            "truncation at {length} decoded"
        );
    }
    let mut trailing = encoded.as_slice().to_vec();
    trailing.push(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&trailing).unwrap_err(),
        GuestConfiguredLaunchesError::TrailingBytes
    );
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&vec![0; MAX_GUEST_CONFIGURED_LAUNCHES_BYTES + 1])
            .unwrap_err(),
        GuestConfiguredLaunchesError::LengthExceeded
    );
}

#[test]
fn configured_launches_reject_header_identity_count_and_digest_mutations() {
    let encoded = catalog().encode().unwrap();
    for (offset, value, expected) in [
        (0, b'X', GuestConfiguredLaunchesError::InvalidMagic),
        (9, 2, GuestConfiguredLaunchesError::UnsupportedSchema),
        (11, 2, GuestConfiguredLaunchesError::UnsupportedVersion),
        (13, 1, GuestConfiguredLaunchesError::InvalidFlags),
        (15, 1, GuestConfiguredLaunchesError::InvalidReserved),
        (95, 1, GuestConfiguredLaunchesError::InvalidReserved),
    ] {
        let mut malformed = encoded.as_slice().to_vec();
        malformed[offset] = value;
        assert_eq!(
            GuestConfiguredLaunchesV1::decode(&malformed).unwrap_err(),
            expected
        );
    }

    let mut bad_realm = encoded.as_slice().to_vec();
    bad_realm[20] = 0xff;
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&bad_realm).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidUtf8
    );
    let mut zero_digest = encoded.as_slice().to_vec();
    zero_digest[60..92].fill(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&zero_digest).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidDigest
    );
    let mut zero_count = encoded.as_slice().to_vec();
    zero_count[92..94].fill(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&zero_count).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidCount
    );
}

#[test]
fn configured_launches_reject_entry_flags_reserved_ids_and_argv() {
    let encoded = catalog().encode().unwrap();
    let (entry_start, flags, argc, reserved) = first_entry_offsets(encoded.as_slice());

    let mut unknown_flags = encoded.as_slice().to_vec();
    unknown_flags[flags..flags + 2].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&unknown_flags).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidFlags
    );
    let mut bad_reserved = encoded.as_slice().to_vec();
    bad_reserved[reserved..reserved + 2].copy_from_slice(&1_u16.to_be_bytes());
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&bad_reserved).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidReserved
    );
    let mut empty_id = encoded.as_slice().to_vec();
    empty_id[entry_start + 4..entry_start + 6].fill(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&empty_id).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidItemId
    );
    let mut invalid_id_utf8 = encoded.as_slice().to_vec();
    invalid_id_utf8[entry_start + 6] = 0xff;
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&invalid_id_utf8).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidUtf8
    );
    let mut empty_argv = encoded.as_slice().to_vec();
    empty_argv[argc..argc + 2].fill(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&empty_argv).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidArgv
    );
    let mut excessive_argc = encoded.as_slice().to_vec();
    excessive_argc[argc..argc + 2].copy_from_slice(&129_u16.to_be_bytes());
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&excessive_argc).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidArgv
    );

    let first_arg_len = reserved + 2;
    let first_arg = first_arg_len + 2;
    let mut empty_program = encoded.as_slice().to_vec();
    empty_program[first_arg_len..first_arg_len + 2].fill(0);
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&empty_program).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidArgv
    );
    let mut excessive_arg_len = encoded.as_slice().to_vec();
    excessive_arg_len[first_arg_len..first_arg_len + 2].copy_from_slice(&4097_u16.to_be_bytes());
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&excessive_arg_len).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidArgv
    );
    let mut nul_argv = encoded.as_slice().to_vec();
    nul_argv[first_arg] = 0;
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&nul_argv).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidArgv
    );
    let mut invalid_argv_utf8 = encoded.as_slice().to_vec();
    invalid_argv_utf8[first_arg] = 0xff;
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&invalid_argv_utf8).unwrap_err(),
        GuestConfiguredLaunchesError::InvalidUtf8
    );
}

#[test]
fn configured_launches_reject_duplicate_ids_and_bound_excesses() {
    let encoded = catalog().encode().unwrap();
    let first_entry_len = usize::try_from(u32::from_be_bytes(
        encoded.as_slice()[96..100].try_into().unwrap(),
    ))
    .unwrap();
    let second_entry = 96 + 4 + first_entry_len;
    let mut duplicate_wire = encoded.as_slice().to_vec();
    duplicate_wire[second_entry + 6..second_entry + 13].copy_from_slice(b"browser");
    assert_eq!(
        GuestConfiguredLaunchesV1::decode(&duplicate_wire).unwrap_err(),
        GuestConfiguredLaunchesError::DuplicateItemId
    );

    let duplicate = GuestConfiguredLaunchesV1::new(
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        [0x44; 32],
        vec![
            entry("duplicate", &["one"], false),
            entry("duplicate", &["two"], true),
        ],
    );
    assert!(matches!(
        duplicate,
        Err(GuestConfiguredLaunchesError::DuplicateItemId)
    ));

    let entries = (0..=MAX_GUEST_CONFIGURED_LAUNCH_ITEMS)
        .map(|index| entry(&format!("item-{index}"), &["program"], false))
        .collect();
    assert!(matches!(
        GuestConfiguredLaunchesV1::new(
            RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
            WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
            [0x44; 32],
            entries,
        ),
        Err(GuestConfiguredLaunchesError::InvalidCount)
    ));
    assert!(
        GuestConfiguredLaunchesV1::new(
            RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
            WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
            [0x44; 32],
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        GuestConfiguredLaunchesV1::new(
            RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
            WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
            [0; 32],
            vec![entry("one", &["program"], false)],
        )
        .is_err()
    );
}

#[test]
fn configured_launches_debug_and_errors_are_redacted() {
    let value = catalog();
    let encoded = value.encode().unwrap();
    for rendered in [
        format!("{value:?}"),
        format!("{:?}", value.entries()[0]),
        format!("{encoded:?}"),
    ] {
        assert!(!rendered.contains("firefox"));
        assert!(!rendered.contains("--private-window"));
        assert!(!rendered.contains("browser"));
        assert!(rendered.contains("REDACTED"));
    }
    let error = GuestConfiguredLaunchesV1::decode(b"private-argv").unwrap_err();
    let rendered = format!("{error:?}");
    assert_eq!(rendered, "guest-configured-launches-invalid-magic");
    assert!(!rendered.contains("private-argv"));
}
