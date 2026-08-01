//! Closed USBIP worker declarations.

/// Long-lived worker class owned by the Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipWorkerClass {
    /// One shared backend per Host.
    HostBackend,
    /// One multiplexed relay per Network.
    NetworkRelay,
    /// One private proxy per attached Binding.
    BindingProxy,
}

/// Closed long-lived Process declaration with no argv, locator, or identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipWorkerDeclaration {
    class: UsbipWorkerClass,
    template: &'static str,
    placement: &'static str,
}

impl UsbipWorkerDeclaration {
    /// Return the signed declaration for one worker class.
    pub const fn for_class(class: UsbipWorkerClass) -> Self {
        match class {
            UsbipWorkerClass::HostBackend => Self {
                class,
                template: "usbip-daemon",
                placement: "host",
            },
            UsbipWorkerClass::NetworkRelay => Self {
                class,
                template: "usbip-relay",
                placement: "host",
            },
            UsbipWorkerClass::BindingProxy => Self {
                class,
                template: "usbip-guest-proxy",
                placement: "guest",
            },
        }
    }

    /// Return the worker class.
    pub const fn class(self) -> UsbipWorkerClass {
        self.class
    }

    /// Return the signed component template name.
    pub const fn template(self) -> &'static str {
        self.template
    }

    /// Return the closed placement class.
    pub const fn placement(self) -> &'static str {
        self.placement
    }
}

/// Binding attachment activation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentActivation {
    /// Reconciliation performs the attachment when dependencies become Ready.
    Declared,
    /// An authorized operator request triggers the attachment.
    Explicit,
}

/// One-shot guest attachment effect, never a long-lived worker Process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentCommand {
    /// Attach through the Binding-owned private Endpoint.
    Attach(AttachmentActivation),
    /// Detach the Binding from the Guest.
    Detach,
}
