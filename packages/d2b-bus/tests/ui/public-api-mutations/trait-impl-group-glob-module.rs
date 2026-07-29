mod deep {
    pub(super) mod aliases {
        pub(crate) struct ComponentSessionAdmission;
        pub(crate) type Admission = ComponentSessionAdmission;
    }
}

use deep::{*};

struct ZoneRegistrar;

impl From<&ZoneRegistrar> for aliases::Admission {
    fn from(_: &ZoneRegistrar) -> Self {
        deep::aliases::ComponentSessionAdmission
    }
}
