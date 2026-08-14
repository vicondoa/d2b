//! Guest-source stream validation.

use crate::{Category, NotificationRequest, SessionEvidence};
use std::collections::BTreeSet;

/// A Guest source bound to an allowlisted category set.
pub struct GuestSource {
    categories: BTreeSet<Category>,
}

impl GuestSource {
    /// Construct a Guest source. An empty category set is never admitted.
    pub fn new(categories: impl IntoIterator<Item = Category>) -> Result<Self, &'static str> {
        let categories = categories.into_iter().collect::<BTreeSet<_>>();
        if categories.is_empty() {
            return Err("notification-category-set-empty");
        }
        Ok(Self { categories })
    }

    /// Validate a request before opening a sink stream.
    pub fn validate(&self, request: &NotificationRequest) -> Result<(), &'static str> {
        if self.categories.contains(&request.category()) {
            Ok(())
        } else {
            Err("notification-category-denied")
        }
    }

    /// Validate a request received over an authenticated Guest source session.
    pub fn validate_authenticated(
        &self,
        session: &SessionEvidence,
        request: &NotificationRequest,
    ) -> Result<(), &'static str> {
        session
            .admit_source()
            .map_err(|_| "notification-source-unauthenticated")?;
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
        let source = GuestSource::new([Category::SystemInfo]).unwrap();
        let request =
            NotificationRequest::new("summary", "body", Category::SystemInfo).unwrap();
        assert_eq!(
            source.validate_authenticated(&test_observer("alice"), &request),
            Err("notification-source-unauthenticated")
        );
        assert!(source
            .validate_authenticated(&test_source("guest"), &request)
            .is_ok());
    }
}
