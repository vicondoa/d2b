struct ComponentSessionAdmission;

#[doc(hidden)]
impl Default for ChainedAdmissionAlias {
    fn default() -> Self {
        ComponentSessionAdmission
    }
}

type AdmissionAlias = ComponentSessionAdmission;
type ChainedAdmissionAlias = AdmissionAlias;
