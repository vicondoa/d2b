mod aliases {
    pub(super) struct ComponentSessionAdmission;
    pub(super) use ComponentSessionAdmission as Admission;
}

use aliases as first;
use first as second;

struct ZoneRegistrar;

#[doc(hidden)]
impl From<&ZoneRegistrar> for second::Admission {
    fn from(_: &ZoneRegistrar) -> Self {
        aliases::ComponentSessionAdmission
    }
}
