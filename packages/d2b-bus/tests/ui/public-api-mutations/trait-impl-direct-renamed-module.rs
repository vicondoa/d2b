mod aliases {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

use aliases as cap;

impl Default for cap::Admission {
    fn default() -> Self {
        aliases::ComponentSessionAdmission
    }
}
