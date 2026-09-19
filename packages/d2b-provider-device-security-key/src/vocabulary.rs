//! The security-key family's declared vocabulary.
//!
//! Every fact here is owned by this crate: the device-class label the
//! privileged hidraw open records, the inventory `busClass` the family's
//! Device rows are selected under, and the udev group the configured FIDO
//! hidraw nodes are granted to. Shared consumers read these constants
//! instead of restating the spellings.

/// The device-class label the privileged FIDO hidraw open records in its
/// audit trail and response body.
pub const SECURITY_KEY_DEVICE_CLASS: &str = "hidraw-fido";

/// The inventory `busClass` the family's Device rows are selected under.
pub const SECURITY_KEY_BUS_CLASS: &str = "hidraw";

/// The udev group the configured FIDO hidraw nodes are granted to.
pub const SECURITY_KEY_UDEV_GROUP: &str = "d2b-security-key";