//! Distro-matrix L3 pin parser/drift policy lint, migrated from
//! `tests/l3-pin-consistency.sh`. Reads the committed pin files under
//! `tests/golden/l3-matrix/` (via the `read_repo_file` / `repo_root` helpers)
//! and asserts each one parses as an ini-flavoured `key = value` file carrying
//! the required keys, a valid `sha256` (the sentinel `placeholder` or a 64-char
//! lowercase-hex digest), an `https://` `image_url`, and
//! `panel_approval_required_for_change = true` (drift requires an ADR).
//!
//! This crate runs only from `tests/tools/rust-workspace-checks.sh` against the real
//! checkout (it is excluded from the hermetic Nix sandbox workspace build), so
//! reading repo files is sound.

use std::collections::BTreeMap;

use d2b_contract_tests::{read_repo_file, repo_root};

const PIN_DIR: &str = "tests/golden/l3-matrix";

const REQUIRED_PINS: &[&str] = &["w3-ubuntu.txt", "w3-fedora.txt", "w3-arch.txt"];

const REQUIRED_KEYS: &[&str] = &[
    "os",
    "release",
    "image_url",
    "sha256",
    "kernel_min",
    "kernel_shipped",
    "cgroup",
    "network_manager",
    "nftables",
    "cloud_hypervisor_min",
    "minijail",
    "panel_approval_required_for_change",
];

/// Parse a pin file's non-comment, non-blank lines into a `key -> value` map,
/// asserting each content line is a `key = value` pair with a lowercase
/// `[a-z0-9_]` key (faithful to the bash gate's `key = value` + key-syntax
/// checks).
fn parse_pin(pin: &str, body: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (raw_key, raw_val) = trimmed
            .split_once('=')
            .unwrap_or_else(|| panic!("l3-pin-consistency: {pin}: malformed line: {trimmed}"));
        let key = raw_key.trim();
        let val = raw_val.trim();
        assert!(
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "l3-pin-consistency: {pin}: invalid key syntax: '{key}'"
        );
        map.insert(key.to_string(), val.to_string());
    }
    map
}

#[test]
fn l3_matrix_pins_parse_and_carry_required_keys() {
    // The pin directory must exist and hold the required pins.
    assert!(
        repo_root().join(PIN_DIR).is_dir(),
        "l3-pin-consistency: pin directory missing: {PIN_DIR}"
    );

    for pin in REQUIRED_PINS {
        let rel = format!("{PIN_DIR}/{pin}");
        let body = read_repo_file(&rel);
        let map = parse_pin(pin, &body);

        for key in REQUIRED_KEYS {
            assert!(
                map.contains_key(*key),
                "l3-pin-consistency: {pin}: missing required key '{key}'"
            );
        }

        // sha256: the `placeholder` sentinel or a 64-char lowercase-hex digest
        // (refuses mojibake or partial digests).
        let sha = &map["sha256"];
        if sha != "placeholder" {
            assert!(
                sha.len() == 64
                    && sha
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "l3-pin-consistency: {pin}: sha256 must be 'placeholder' or 64-char lowercase hex, got: '{sha}'"
            );
        }

        // image_url must be https://.
        let url = &map["image_url"];
        assert!(
            url.starts_with("https://"),
            "l3-pin-consistency: {pin}: image_url must be https://: '{url}'"
        );

        // Drift requires an ADR: panel approval is mandatory.
        let panel = &map["panel_approval_required_for_change"];
        assert_eq!(
            panel, "true",
            "l3-pin-consistency: {pin}: panel_approval_required_for_change must be 'true' (drift requires ADR), got: '{panel}'"
        );
    }
}
