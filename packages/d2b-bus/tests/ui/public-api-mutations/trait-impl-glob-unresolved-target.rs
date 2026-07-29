mod facade {
    pub(super) use unresolved_facade::aliases;
}

use facade::*;

struct LocalInput;

impl From<LocalInput> for aliases::Admission {
    fn from(_: LocalInput) -> Self {
        unreachable!()
    }
}
