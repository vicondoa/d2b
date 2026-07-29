mod capability {
    pub(super) struct ComponentSessionAdmission;
}

mod facade {
    pub(super) mod nested {
        pub(crate) use super::super::capability::ComponentSessionAdmission as Admission;
    }
}

mod exports {
    pub(super) use super::facade as wrapper;
}

use exports::*;

struct LocalInput;

impl From<LocalInput> for wrapper::nested::Admission {
    fn from(_: LocalInput) -> Self {
        capability::ComponentSessionAdmission
    }
}
