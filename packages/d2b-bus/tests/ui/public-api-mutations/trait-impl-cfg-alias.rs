struct ComponentSessionAdmission;

#[cfg(all())]
type AdmissionAlias = ComponentSessionAdmission;

impl Default for AdmissionAlias {
    fn default() -> Self {
        ComponentSessionAdmission
    }
}
