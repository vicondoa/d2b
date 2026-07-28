use d2b_bus::{UnixSubjectConfig, ZoneRegistrar};

fn inject(registrar: &ZoneRegistrar, subject: UnixSubjectConfig) {
    registrar.register_unix_subject(subject).unwrap();
}
