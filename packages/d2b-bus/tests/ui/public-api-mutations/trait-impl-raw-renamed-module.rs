mod r#aliases {
    pub(super) struct r#ComponentSessionAdmission;
    pub(super) type r#Admission = r#ComponentSessionAdmission;
}

use r#aliases as r#cap;

impl Default for r#cap::r#Admission {
    fn default() -> Self {
        r#aliases::r#ComponentSessionAdmission
    }
}
