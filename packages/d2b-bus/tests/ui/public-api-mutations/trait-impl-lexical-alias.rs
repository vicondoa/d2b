struct ComponentSessionAdmission;
struct Harmless;

fn capability_scope() {
    type AdmissionAlias = ComponentSessionAdmission;

    impl Default for AdmissionAlias {
        fn default() -> Self {
            ComponentSessionAdmission
        }
    }
}

fn harmless_scope() {
    type AdmissionAlias = Harmless;

    impl Default for AdmissionAlias {
        fn default() -> Self {
            Harmless
        }
    }
}
