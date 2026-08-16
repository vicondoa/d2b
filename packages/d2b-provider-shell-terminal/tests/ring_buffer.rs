use d2b_provider_shell_terminal::OutputRing;

#[test]
fn ring_evicts_oldest_output_and_debug_never_reveals_terminal_bytes() {
    let mut ring = OutputRing::new(4096).unwrap();
    ring.append(&vec![b'a'; 4096]);
    ring.append(b"secret-terminal-bytes");

    assert_eq!(ring.len(), 4096);
    assert_eq!(ring.evicted_bytes(), 21);
    let replay = ring.tail(21);
    assert_eq!(replay.bytes(), b"secret-terminal-bytes");
    assert!(replay.was_truncated());
    assert!(!format!("{replay:?}").contains("secret-terminal-bytes"));
}
