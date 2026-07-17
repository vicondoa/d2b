//! Bounds for decoded clipboard-picker service projections.
//!
//! ComponentSession owns packet framing and authentication. This module only
//! validates the bounded projection handed to picker policy after the generated
//! service request has been decoded.

use thiserror::Error;

use crate::protocol::{MAX_OFFERS_PER_PAGE, MAX_THUMBNAIL_BYTES};

pub const MAX_PICKER_PACKET_BYTES: usize = 512 * 1024;
pub const MAX_PICKER_ATTACHMENTS: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerProjectionBounds {
    pub encoded_bytes: usize,
    pub offer_count: usize,
    pub thumbnail_bytes: usize,
    pub attachment_count: usize,
}

impl PickerProjectionBounds {
    pub fn validate(self) -> Result<(), ProjectionError> {
        if self.encoded_bytes == 0
            || self.encoded_bytes > MAX_PICKER_PACKET_BYTES
            || self.offer_count > MAX_OFFERS_PER_PAGE
            || self.thumbnail_bytes > MAX_THUMBNAIL_BYTES
            || self.attachment_count != MAX_PICKER_ATTACHMENTS
        {
            return Err(ProjectionError::OutOfBounds);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ProjectionError {
    #[error("clipboard-picker-projection-out-of-bounds")]
    OutOfBounds,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_projection_is_bounded_and_attachment_free() {
        PickerProjectionBounds {
            encoded_bytes: 1024,
            offer_count: 8,
            thumbnail_bytes: 4096,
            attachment_count: 0,
        }
        .validate()
        .unwrap();

        for invalid in [
            PickerProjectionBounds {
                encoded_bytes: 0,
                offer_count: 0,
                thumbnail_bytes: 0,
                attachment_count: 0,
            },
            PickerProjectionBounds {
                encoded_bytes: MAX_PICKER_PACKET_BYTES + 1,
                offer_count: 0,
                thumbnail_bytes: 0,
                attachment_count: 0,
            },
            PickerProjectionBounds {
                encoded_bytes: 1,
                offer_count: 0,
                thumbnail_bytes: 0,
                attachment_count: 1,
            },
        ] {
            assert_eq!(invalid.validate(), Err(ProjectionError::OutOfBounds));
        }
    }
}
