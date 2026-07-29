struct ComponentSessionAdmission;

const PRIVATE_ALIAS_PATH: &str = "/home/alice/private/alias.rs";

type AdmissionAlias = [ComponentSessionAdmission; { PRIVATE_ALIAS_PATH.len() }];

impl Default for AdmissionAlias {
    fn default() -> Self {
        [ComponentSessionAdmission; PRIVATE_ALIAS_PATH.len()]
    }
}
