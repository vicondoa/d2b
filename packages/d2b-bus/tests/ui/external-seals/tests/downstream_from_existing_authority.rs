use d2b_bus::ComponentSessionAdmission;

struct LocalInput(ComponentSessionAdmission);

impl From<LocalInput> for ComponentSessionAdmission {
    fn from(input: LocalInput) -> Self {
        input.0
    }
}

#[test]
fn downstream_from_can_only_return_held_authority() {
    let _ = core::mem::size_of::<LocalInput>();
}
