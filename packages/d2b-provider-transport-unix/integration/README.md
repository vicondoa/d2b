# Integration fixtures

The focused Rust integration surface lives in `../tests/scaffold.rs`. It uses
real AF_UNIX socketpairs to prove portal admission, `SO_PASSCRED`, close-on-exec,
route attachment refusal, and owned-monitor finalization without touching host
state.
