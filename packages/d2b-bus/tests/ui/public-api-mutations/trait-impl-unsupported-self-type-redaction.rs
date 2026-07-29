struct ComponentSessionAdmission;

const PRIVATE_SELF_TYPE_PATH: &str = "/home/alice/private/self-type.rs";

impl Default
    for [ComponentSessionAdmission; { PRIVATE_SELF_TYPE_PATH.len() }]
{
    fn default() -> Self {
        [ComponentSessionAdmission; PRIVATE_SELF_TYPE_PATH.len()]
    }
}
