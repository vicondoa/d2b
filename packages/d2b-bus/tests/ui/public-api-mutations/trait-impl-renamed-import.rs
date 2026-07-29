mod declarations {
    pub(super) struct ComponentSessionAdmission;
    pub(super) type AdmissionAlias = ComponentSessionAdmission;
}

use declarations::AdmissionAlias as RenamedAdmission;

#[doc(hidden)]
impl Default for RenamedAdmission {
    fn default() -> Self {
        declarations::ComponentSessionAdmission
    }
}
