use std::{env, fs, path::PathBuf};

use nixling_core::privileges::PrivilegesJson;

pub fn load_privileges_fixture_from_env() -> PrivilegesJson {
    let fixtures = env::var_os("NL_FIXTURES")
        .unwrap_or_else(|| panic!("NL_FIXTURES must point to the fixture-smoke output directory"));
    let path = PathBuf::from(fixtures).join("privileges.json");
    let json = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read privileges fixture at {}: {err}",
            path.display()
        )
    });

    serde_json::from_str(&json).unwrap_or_else(|err| {
        panic!(
            "failed to parse privileges fixture at {} as nixling_core::privileges::PrivilegesJson: {err}",
            path.display()
        )
    })
}
