mod deep {
    pub(super) mod aliases {
        pub(super) struct ComponentSessionAdmission;
        pub(super) type Admission = ComponentSessionAdmission;
    }
}

use deep::aliases;

struct ZoneRegistrar;

impl From<&ZoneRegistrar> for aliases::Admission {
    fn from(_: &ZoneRegistrar) -> Self {
        deep::aliases::ComponentSessionAdmission
    }
}
