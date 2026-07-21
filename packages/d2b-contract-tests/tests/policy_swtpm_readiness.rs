//! Contract test: the canonical `swtpm` role process uses an active-listener
//! readiness predicate rather than accepting a stale socket inode.

use d2b_contract_tests::read_repo_file;

#[test]
fn swtpm_readiness_uses_unix_socket_listening() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    let start = emitter
        .find(r#"else if role.roleKind == "swtpm" then mkNode {"#)
        .expect("processes-json.nix must compose a canonical swtpm role node");
    let tail = &emitter[start..];
    let end = tail
        .find("\n    else if ")
        .expect("swtpm role node must be followed by another role branch");
    let block = &tail[..end];

    assert!(
        block.contains(r#"id = role.roleId;"#) && block.contains(r#"role = role.processRole;"#),
        "swtpm process must use the normalized role id and process role"
    );
    assert!(
        block.contains(r#"ready = [ (socketListening "${runtime}/tpm.sock") ];"#),
        "swtpm readiness must probe the canonical role runtime listener"
    );
    assert!(
        !block.contains("socketExists"),
        "swtpm readiness must not accept a stale socket inode"
    );
}
