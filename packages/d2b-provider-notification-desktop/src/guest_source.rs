//! Guest-source stream validation.

use crate::{Category, NotificationRequest};
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
}

impl core::fmt::Debug for GuestSource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestSource")
            .field("category_count", &self.categories.len())
            .finish()
    }
}
