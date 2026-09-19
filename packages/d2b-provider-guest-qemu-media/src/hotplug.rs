//! QEMU-media hotplug scaffold planning.
//!
//! The Provider owns the hotplug vocabulary: the QMP command sequence for
//! an attach or detach and the opaque blockdev/device ids derived from a
//! media ref. The host crate keeps only the generic USB helpers; the
//! broker's privileged media kernel keeps its own committed view of this
//! scaffold because the broker is pinned provider-free.

use crate::types::validate_token;

/// The hotplug action one scaffold plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaHotplugAction {
    /// Attach a block backend and guest device.
    Attach,
    /// Detach the guest device and block backend.
    Detach,
}

impl QemuMediaHotplugAction {
    /// The QMP command names the action dispatches, in order.
    pub fn qmp_commands(self) -> &'static [&'static str] {
        match self {
            Self::Attach => &["blockdev-add", "device_add"],
            Self::Detach => &["device_del", "blockdev-del"],
        }
    }
}

/// One planned hotplug transaction: the opaque ids and QMP command names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuMediaHotplugScaffold {
    /// Opaque media ref the transaction targets.
    pub media_ref: String,
    /// Slot the media occupies (e.g. `boot`).
    pub slot: String,
    /// QMP block node name.
    pub blockdev_id: String,
    /// QEMU device id.
    pub device_id: String,
    /// QMP command names in dispatch order.
    pub qmp_commands: Vec<String>,
}

/// Scaffold planning failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaHotplugScaffoldError {
    /// The media ref is not a bounded lowercase token.
    InvalidMediaRef,
    /// The slot is empty.
    EmptySlot,
    /// The slot is not a bounded lowercase token.
    InvalidSlot,
}

/// Plan one hotplug transaction from an opaque media ref and slot.
pub fn qemu_media_hotplug_scaffold(
    media_ref: &str,
    slot: &str,
    action: QemuMediaHotplugAction,
) -> Result<QemuMediaHotplugScaffold, QemuMediaHotplugScaffoldError> {
    if !validate_token(media_ref) {
        return Err(QemuMediaHotplugScaffoldError::InvalidMediaRef);
    }
    if slot.is_empty() {
        return Err(QemuMediaHotplugScaffoldError::EmptySlot);
    }
    if slot.len() > 63
        || !slot
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(QemuMediaHotplugScaffoldError::InvalidSlot);
    }
    Ok(QemuMediaHotplugScaffold {
        media_ref: media_ref.to_owned(),
        slot: slot.to_owned(),
        blockdev_id: format!("d2b-media-{media_ref}"),
        device_id: format!("d2b-usb-{media_ref}"),
        qmp_commands: action
            .qmp_commands()
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qmp_scaffold_uses_only_opaque_ref_derived_ids() {
        let plan =
            qemu_media_hotplug_scaffold("installer-usb", "cdrom", QemuMediaHotplugAction::Attach)
                .expect("scaffold");

        assert_eq!(plan.blockdev_id, "d2b-media-installer-usb");
        assert_eq!(plan.device_id, "d2b-usb-installer-usb");
        assert_eq!(plan.qmp_commands, ["blockdev-add", "device_add"]);
    }

    #[test]
    fn qmp_scaffold_rejects_path_like_refs_and_slots() {
        assert!(matches!(
            qemu_media_hotplug_scaffold(
                "/dev/disk/by-id/secret",
                "cdrom",
                QemuMediaHotplugAction::Attach
            ),
            Err(QemuMediaHotplugScaffoldError::InvalidMediaRef)
        ));
        assert!(matches!(
            qemu_media_hotplug_scaffold(
                "installer-usb",
                "../cdrom",
                QemuMediaHotplugAction::Attach
            ),
            Err(QemuMediaHotplugScaffoldError::InvalidSlot)
        ));
    }
}