mod aliases {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

#[cfg(all())]
use aliases::Admission;

impl Default for Admission {
    fn default() -> Self {
        aliases::ComponentSessionAdmission
    }
}
