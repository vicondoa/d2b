mod aliases {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

use aliases::{self as cap};

impl Default for cap::Admission {
    fn default() -> Self {
        aliases::ComponentSessionAdmission
    }
}
