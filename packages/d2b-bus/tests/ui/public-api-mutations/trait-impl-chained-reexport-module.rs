mod aliases {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

mod reexports {
    pub(super) use super::aliases as first;
}

use reexports::first as second;

struct LocalInput;

impl From<LocalInput> for second::Admission {
    fn from(_: LocalInput) -> Self {
        aliases::ComponentSessionAdmission
    }
}
