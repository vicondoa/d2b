struct ComponentSessionAdmission;

type AdmissionAlias<T> = T;

impl Default for AdmissionAlias<ComponentSessionAdmission> {
    fn default() -> Self {
        ComponentSessionAdmission
    }
}
