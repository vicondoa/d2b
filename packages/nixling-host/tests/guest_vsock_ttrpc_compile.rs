#![forbid(unsafe_code)]

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_vsock_ttrpc {
    use ttrpc::r#async::{
        transport::{Listener, Socket},
        Server,
    };
    use tokio::net::UnixStream;

    const GUEST_CONTROL_PORT: u32 = 14318;

    fn guest_listener_addr(port: u32) -> String {
        format!("vsock://-1:{port}")
    }

    fn bind_guest_vsock_server(port: u32) -> std::io::Result<Server> {
        let listener = Listener::bind(guest_listener_addr(port))?;
        Ok(Server::new().add_listener(listener))
    }

    fn wrap_post_connect_stream(stream: UnixStream) -> Socket {
        Socket::new(stream)
    }

    #[test]
    fn compiles_safe_ttrpc_af_vsock_server_shape() {
        let _bind: fn(u32) -> std::io::Result<Server> = bind_guest_vsock_server;
        let _wrap: fn(UnixStream) -> Socket = wrap_post_connect_stream;

        assert_eq!(guest_listener_addr(GUEST_CONTROL_PORT), "vsock://-1:14318");
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
#[test]
fn guest_vsock_ttrpc_compile_proof_is_linux_only() {}
