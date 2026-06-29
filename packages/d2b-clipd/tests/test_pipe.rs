#[test]
fn test_pipe_cloexec() {
    let (read_fd, write_fd) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    let flags = rustix::io::fcntl_getfd(&read_fd).unwrap();
    assert!(
        flags.contains(rustix::io::FdFlags::CLOEXEC),
        "materialization read pipe must be CLOEXEC"
    );
    let flags = rustix::io::fcntl_getfd(&write_fd).unwrap();
    assert!(
        flags.contains(rustix::io::FdFlags::CLOEXEC),
        "materialization write pipe must be CLOEXEC"
    );
}
