//! Guest-source stream validation.

use crate::{Category, GuestSourceConfig, NotificationRequest, SessionEvidence};
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use std::collections::BTreeSet;

/// A Guest source bound to an allowlisted category set.
pub struct GuestSource {
    source_ref: ResourceRef,
    zone: ZoneId,
    categories: BTreeSet<Category>,
    generation: u64,
}

impl GuestSource {
    /// Construct a configured Guest source from Core-owned configuration and
    /// the current authenticated reconnect generation.
    pub fn from_config_at_generation(
        config: &GuestSourceConfig,
        generation: u64,
    ) -> Result<Self, crate::ProviderError> {
        if generation == 0 {
            return Err(crate::ProviderError::SourceGenerationInvalid);
        }
        Ok(Self {
            source_ref: config.source_ref().clone(),
            zone: config.zone().clone(),
            categories: config.categories().clone(),
            generation,
        })
    }

    /// Construct an unbound source for unit tests only.
    #[cfg(test)]
    pub fn new(categories: impl IntoIterator<Item = Category>) -> Result<Self, crate::ProviderError> {
        let categories = categories.into_iter().collect::<BTreeSet<_>>();
        if categories.is_empty() {
            return Err(crate::ProviderError::CategorySetEmpty);
        }
        Ok(Self {
            source_ref: ResourceRef::parse("Guest/test").unwrap(),
            zone: ZoneId::parse("test").unwrap(),
            categories,
            generation: 1,
        })
    }

    /// Validate a request before opening a sink stream.
    pub fn validate(&self, request: &NotificationRequest) -> Result<(), crate::ProviderError> {
        if self.categories.contains(&request.category()) {
            Ok(())
        } else {
            Err(crate::ProviderError::CategoryDenied)
        }
    }

    /// Validate a request received over an authenticated Guest source session.
    pub fn validate_authenticated(
        &self,
        session: &SessionEvidence,
        request: &NotificationRequest,
    ) -> Result<(), crate::ProviderError> {
        session
            .admit_source()
            .map_err(|_| crate::ProviderError::SourceUnauthenticated)?;
        if session.subject_ref() != &self.source_ref || session.zone() != &self.zone {
            return Err(crate::ProviderError::SourceBindingMismatch);
        }
        if session.generation() != self.generation {
            return Err(crate::ProviderError::SourceStaleGeneration);
        }
        self.validate(request)
    }
}

impl core::fmt::Debug for GuestSource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestSource")
            .field("category_count", &self.categories.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::{test_observer, test_source};

    #[test]
    fn authenticated_source_validation_rejects_observer_reuse() {
        let config = GuestSourceConfig::new(
            ResourceRef::parse("Guest/guest").unwrap(),
            ZoneId::parse("work").unwrap(),
            [Category::SystemInfo],
        )
        .unwrap();
        let source = GuestSource::from_config_at_generation(&config, 1).unwrap();
        let request = NotificationRequest::new("summary", "body", Category::SystemInfo).unwrap();
        assert_eq!(
            source.validate_authenticated(&test_observer("alice"), &request),
            Err(crate::ProviderError::SourceUnauthenticated)
        );
        assert!(
            source
                .validate_authenticated(&test_source("guest"), &request)
                .is_ok()
        );
        assert_eq!(
            source.validate_authenticated(&crate::admission::test_source("other"), &request),
            Err(crate::ProviderError::SourceBindingMismatch)
        );
        assert_eq!(
            source.validate_authenticated(&crate::admission::test_source_at("guest", 2), &request),
            Err(crate::ProviderError::SourceStaleGeneration)
        );
    }
}
