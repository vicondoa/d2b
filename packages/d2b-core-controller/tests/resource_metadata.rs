use d2b_core_controller::resource_store::{
    ManagedBy, PersistedResourceMetadata, ResourceMetadataError, managed_by_json, managed_by_str,
    parse_managed_by, parse_managed_by_json,
};

#[test]
fn managed_by_closed_enum_round_trips_each_persisted_value() {
    for value in [
        ManagedBy::Configuration,
        ManagedBy::Controller,
        ManagedBy::Api,
    ] {
        assert_eq!(parse_managed_by(managed_by_str(value)), Ok(value));
        assert_eq!(parse_managed_by_json(managed_by_json(value)), Ok(value));
    }
    assert_eq!(
        parse_managed_by("operator"),
        Err(ResourceMetadataError::UnknownManagedBy)
    );
    assert_eq!(
        parse_managed_by_json("\"operator\""),
        Err(ResourceMetadataError::UnknownManagedBy)
    );
}

#[test]
fn configuration_generation_is_store_owned_and_owner_scoped() {
    let configured = PersistedResourceMetadata::configuration(7).unwrap();
    assert_eq!(configured.managed_by(), ManagedBy::Configuration);
    assert_eq!(configured.configuration_generation(), Some(7));

    assert_eq!(
        PersistedResourceMetadata::configuration(0),
        Err(ResourceMetadataError::ConfigurationGenerationMissing)
    );
    assert_eq!(
        PersistedResourceMetadata::new(ManagedBy::Controller, Some(7), None),
        Err(ResourceMetadataError::ConfigurationGenerationUnexpected)
    );
    assert_eq!(
        PersistedResourceMetadata::new(ManagedBy::Api, Some(7), None),
        Err(ResourceMetadataError::ConfigurationGenerationUnexpected)
    );
}
