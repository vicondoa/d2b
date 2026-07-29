fn install() {
    mod deep {
        pub mod aliases {
            pub struct ComponentSessionAdmission;
        }
    }

    use deep::*;

    struct LocalInput;

    impl From<LocalInput> for aliases::ComponentSessionAdmission {
        fn from(_: LocalInput) -> Self {
            deep::aliases::ComponentSessionAdmission
        }
    }
}
