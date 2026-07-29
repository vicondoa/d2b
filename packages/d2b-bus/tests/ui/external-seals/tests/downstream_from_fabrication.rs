use d2b_bus::ComponentSessionAdmission;

struct LocalInput;

impl From<LocalInput> for ComponentSessionAdmission {
    fn from(_input: LocalInput) -> Self {
        ComponentSessionAdmission {
            identity: unreachable!(),
        }
    }
}

fn main() {}
