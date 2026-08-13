use d2b_contracts::v3::ActivationMode;
use d2b_host::host_generation::{
    ActivationHelperOutcome, ActivationHelperProtocolError, ActivationHelperRequest,
    ActivationHelperResponse, encode_response, parse_request,
};

#[test]
fn helper_json_is_bounded_and_redacts_store_details() {
    let request = br#"{"systemArtifactId":"dev-vm-system","activationMode":"switch"}"#;
    let parsed = parse_request(request).unwrap();
    assert_eq!(parsed.activation_mode, ActivationMode::Switch);
    let response = encode_response(ActivationHelperResponse {
        outcome: ActivationHelperOutcome::Succeeded,
    })
    .unwrap();
    assert_eq!(response, br#"{"outcome":"succeeded"}"#);
    assert!(!String::from_utf8(response).unwrap().contains("/nix/store"));
}

#[test]
fn helper_json_rejects_paths_unknown_fields_and_oversize() {
    assert_eq!(
        parse_request(br#"{"systemArtifactId":"/nix/store/x","activationMode":"switch"}"#)
            .unwrap_err(),
        ActivationHelperProtocolError::ArtifactIdInvalid
    );
    assert_eq!(
        parse_request(br#"{"systemArtifactId":"dev","activationMode":"switch","path":"/x"}"#)
            .unwrap_err(),
        ActivationHelperProtocolError::InvalidJson
    );
    assert_eq!(
        parse_request(&vec![b'x'; 2049]).unwrap_err(),
        ActivationHelperProtocolError::TooLarge
    );
    assert!(
        ActivationHelperRequest {
            system_artifact_id: "dev".to_owned(),
            activation_mode: ActivationMode::Switch,
        }
        .validate()
        .is_ok()
    );
}
