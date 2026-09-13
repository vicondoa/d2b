//! Registration sources that admit a driver for one resource type.

use bitflags::bitflags;

bitflags! {
    /// The registration sources that admit one resource type's driver.
    ///
    /// The mask is the registry's registration contract: a source whose bit
    /// is absent never admits the driver for that type, and a type that can
    /// only be registered late is required when the plane opens.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AllowedSources: u8 {
        /// The type belongs to the built-in foundation registration.
        const BUILTIN = 0b001;
        /// The type is admitted by the zone startup sequence.
        const STARTUP = 0b010;
        /// The type is admitted by late in-process registration once the
        /// plane is already open.
        const RUNTIME = 0b100;
    }
}

impl AllowedSources {
    /// Whether the resource type must be registered when the plane opens.
    ///
    /// A type whose mask lacks [`AllowedSources::RUNTIME`] cannot be
    /// registered late, so the presence obligation applies to it: startup
    /// fails, naming the driver, when the type is not registered by the time
    /// the plane opens.
    pub const fn requires_plane_registration(self) -> bool {
        !self.contains(Self::RUNTIME)
    }
}

#[cfg(test)]
mod tests {
    use super::AllowedSources;

    /// A mask without the RUNTIME bit carries the presence obligation no
    /// matter which of the other bits it sets.
    #[test]
    fn a_mask_without_runtime_requires_plane_registration() {
        assert!(AllowedSources::BUILTIN.requires_plane_registration());
        assert!(AllowedSources::STARTUP.requires_plane_registration());
        assert!(
            AllowedSources::BUILTIN
                .union(AllowedSources::STARTUP)
                .requires_plane_registration()
        );
        assert!(AllowedSources::empty().requires_plane_registration());
    }

    /// Any mask that admits runtime registration is exempt from the presence
    /// obligation.
    #[test]
    fn a_mask_with_runtime_does_not_require_plane_registration() {
        assert!(!AllowedSources::RUNTIME.requires_plane_registration());
        assert!(
            !AllowedSources::BUILTIN
                .union(AllowedSources::RUNTIME)
                .requires_plane_registration()
        );
        assert!(!AllowedSources::all().requires_plane_registration());
    }

    /// The three sources are independent bits: they combine without overlap
    /// and together cover the full mask.
    #[test]
    fn the_three_bits_combine_without_overlap() {
        assert_eq!(
            AllowedSources::BUILTIN
                .union(AllowedSources::STARTUP)
                .union(AllowedSources::RUNTIME),
            AllowedSources::all()
        );
        assert_eq!(AllowedSources::all().bits(), 0b111);
        assert!(!AllowedSources::BUILTIN.intersects(AllowedSources::STARTUP));
        assert!(!AllowedSources::BUILTIN.intersects(AllowedSources::RUNTIME));
        assert!(!AllowedSources::STARTUP.intersects(AllowedSources::RUNTIME));
    }
}
