use d2b_contracts::v3::{ResourceUid, SchemaFingerprint, SchemaVersion, VolumeStateSchemaId};
use d2b_provider_volume_local::marker::{
    MarkerBinding, MarkerDisposition, MarkerError, MarkerStore, VerifiedMarkerFile,
    VolumeRootIdentity, provision_marker, verify_marker,
};

#[derive(Default)]
struct MemoryMarkerStore {
    bytes: Option<Vec<u8>>,
}

impl MarkerStore for MemoryMarkerStore {
    fn read_marker(
        &mut self,
        _volume_uid: &ResourceUid,
    ) -> Result<Option<VerifiedMarkerFile>, MarkerError> {
        Ok(self
            .bytes
            .clone()
            .map(VerifiedMarkerFile::from_verified_regular_file))
    }

    fn create_marker_exclusive(
        &mut self,
        _volume_uid: &ResourceUid,
        bytes: &[u8],
    ) -> Result<(), MarkerError> {
        if self.bytes.is_some() {
            return Err(MarkerError::MarkerWriteFailed);
        }
        self.bytes = Some(bytes.to_vec());
        Ok(())
    }
}

fn binding(root: VolumeRootIdentity) -> MarkerBinding {
    MarkerBinding::new(
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap(),
        root,
        VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
        SchemaVersion::new(1, 0).unwrap(),
        SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
    )
}

#[test]
fn missing_but_previously_provisioned_root_is_never_silently_recreated() {
    let root = VolumeRootIdentity {
        device: 31,
        inode: 47,
    };
    let expected = binding(root);
    let mut store = MemoryMarkerStore::default();

    assert_eq!(
        verify_marker(&mut store, None, &expected),
        Ok(MarkerDisposition::Unprovisioned)
    );
    provision_marker(&mut store, &expected).expect("first marker provision succeeds");

    assert_eq!(
        verify_marker(&mut store, None, &expected),
        Err(MarkerError::PreviouslyProvisionedStateMissing)
    );
    assert!(
        store.bytes.is_some(),
        "failed verification preserves marker evidence"
    );
}

#[test]
fn root_created_before_marker_commit_is_not_adopted() {
    let root = VolumeRootIdentity {
        device: 31,
        inode: 47,
    };
    let expected = binding(root);
    let mut store = MemoryMarkerStore::default();
    assert_eq!(
        verify_marker(&mut store, Some(root), &expected),
        Err(MarkerError::MarkerMissing)
    );
}

#[test]
fn every_visible_provision_crash_state_is_classified() {
    let root = VolumeRootIdentity {
        device: 31,
        inode: 47,
    };
    let expected = binding(root);

    let mut before_root = MemoryMarkerStore::default();
    assert_eq!(
        verify_marker(&mut before_root, None, &expected),
        Ok(MarkerDisposition::Unprovisioned)
    );

    let mut after_root = MemoryMarkerStore::default();
    assert_eq!(
        verify_marker(&mut after_root, Some(root), &expected),
        Err(MarkerError::MarkerMissing)
    );

    let mut partial_marker = MemoryMarkerStore {
        bytes: Some(br#"{"version":1"#.to_vec()),
    };
    assert_eq!(
        verify_marker(&mut partial_marker, Some(root), &expected),
        Err(MarkerError::MarkerInvalid)
    );

    let mut committed = MemoryMarkerStore::default();
    provision_marker(&mut committed, &expected).unwrap();
    assert_eq!(
        verify_marker(&mut committed, Some(root), &expected),
        Ok(MarkerDisposition::Verified)
    );

    assert_eq!(
        verify_marker(&mut committed, None, &expected),
        Err(MarkerError::PreviouslyProvisionedStateMissing)
    );
}

#[test]
fn replacement_and_marker_mismatch_are_rejected() {
    let root = VolumeRootIdentity {
        device: 31,
        inode: 47,
    };
    let expected = binding(root);
    let mut store = MemoryMarkerStore::default();
    provision_marker(&mut store, &expected).unwrap();

    assert_eq!(
        verify_marker(
            &mut store,
            Some(VolumeRootIdentity {
                device: 31,
                inode: 48,
            }),
            &expected,
        ),
        Err(MarkerError::RootReplaced)
    );

    let marker = store.bytes.as_mut().unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(marker).unwrap();
    value["installedSchemaVersion"] = serde_json::json!("2.0");
    *marker = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        verify_marker(&mut store, Some(root), &expected),
        Err(MarkerError::MarkerInvalid)
    );
}
