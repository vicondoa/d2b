mod deep {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type Admission = ComponentSessionAdmission;
}

use deep as facade;
use facade::*;

struct LocalInput;

impl From<LocalInput> for Admission {
    fn from(_: LocalInput) -> Self {
        deep::ComponentSessionAdmission
    }
}
