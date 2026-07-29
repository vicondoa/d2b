use d2b_bus::ComponentSessionAdmission;
use d2b_session::{AuthenticatedComponentSession, SessionAcceptor};
use d2b_session_unix::VerifiedUnixPeer;

#[test]
fn capability_types_are_available() {
    let _ = core::mem::size_of::<Option<ComponentSessionAdmission>>();
    let _ = core::mem::size_of::<Option<VerifiedUnixPeer>>();
    let _ = core::mem::size_of::<Option<SessionAcceptor<()>>>();
    let _ = core::mem::size_of::<Option<AuthenticatedComponentSession<()>>>();
}
