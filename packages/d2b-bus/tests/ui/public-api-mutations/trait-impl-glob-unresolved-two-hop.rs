mod first {
    pub(super) use unresolved_facade::aliases;
}

mod second {
    pub(super) use super::first::*;
}

use second::*;

struct LocalInput;

impl From<LocalInput> for aliases::Admission {
    fn from(_: LocalInput) -> Self {
        unreachable!()
    }
}
