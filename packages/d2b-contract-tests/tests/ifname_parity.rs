use d2b_contract_tests::load_bundle_resolver_from_env;
use d2b_host::ifname::{DEFAULT_PREFIX, looks_d2b_owned};

#[test]
fn rendered_host_json_ifname_mappings_pass_looks_d2b_owned() {
    let resolver = load_bundle_resolver_from_env();
    let mappings = &resolver.host.if_name_mappings;

    assert!(
        !mappings.is_empty(),
        "rendered fixture host.json has empty `ifNameMappings`; emitter regression suspected"
    );

    let mut violations: Vec<String> = Vec::new();
    for row in mappings {
        let name = row.derived_ifname.as_str();
        if !looks_d2b_owned(name, DEFAULT_PREFIX) {
            violations.push(format!(
                "{name} is not accepted by looks_d2b_owned(prefix={DEFAULT_PREFIX:?})"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Nix-emitted ifnames failed Rust looks_d2b_owned ({} of {}):\n  {}",
        violations.len(),
        mappings.len(),
        violations.join("\n  ")
    );

    eprintln!(
        "rendered_host_json_ifname_mappings_pass_looks_d2b_owned: {} derivedIfname values accepted",
        mappings.len()
    );
}
