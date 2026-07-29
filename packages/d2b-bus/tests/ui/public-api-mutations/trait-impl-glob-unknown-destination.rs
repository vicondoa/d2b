use unresolved_facade::*;

struct LocalInput;

impl From<LocalInput> for aliases::Admission {
    fn from(_: LocalInput) -> Self {
        unreachable!()
    }
}
