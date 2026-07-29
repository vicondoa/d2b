use aliases as cap;

mod aliases {
    pub(super) use super::later::Admission;
}

mod later {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

struct LocalInput;

impl From<LocalInput> for cap::Admission {
    fn from(_: LocalInput) -> Self {
        later::ComponentSessionAdmission
    }
}
