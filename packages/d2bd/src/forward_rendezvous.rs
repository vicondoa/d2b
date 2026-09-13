//! The forwarding rendezvous: the endpoint the broker dials.
//!
//! The broker is a seqpacket listener and links no provider crate, so a
//! committed operation's handler cannot run there. The dispatch step of its
//! envelope forwards instead: one validated, authorized invocation crosses to
//! the process that declares the handler, which answers with the handler's
//! canonical result or its own refusal code. This module is the receiving
//! end - a seqpacket listener the daemon owns, beside the broker socket - and
//! the routing it performs: the call names its Zone and its operation, and
//! the Zone's started providers resolve it to the provider that declared the
//! operation, whose toolkit envelope runs the handler.
//!
//! Two properties are structural. The listener binds only when the
//! environment names a path, so a daemon with no forwarding peer is
//! fail-closed exactly as the broker is, and the broker's own no-default
//! stance keeps the two ends in agreement. And resolution runs over the
//! *started* providers' own descriptor tables - the handler table the plane
//! registered - so an operation no started provider declares is refused by
//! name before any handler runs.
//!
//! The forwarded hop was authorized at the broker against the committed rows;
//! the carrier deliberately carries no caller identity, because a second
//! identity on this side would be a second authority to keep in sync. The
//! provider-side envelope therefore runs each declared operation under the
//! declaring provider's own reference, which is the one caller fact this
//! process owns: a provider may run the handlers it declared, and the
//! envelope refuses every caller it holds no grant for.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use d2b_contracts_broker::FORWARD_SOCKET_ENV;
use d2b_contracts_broker::broker_wire::{
    ForwardOperationOutcome, ForwardOperationRequest, ForwardOperationResponse,
};
use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_provider_toolkit::operations::UNCOMMITTED_OPERATION;
use d2bd_runtime::concurrency::{ConnSemaphore, DEFAULT_MAX_INFLIGHT_CONNECTIONS};
use d2bd_runtime::runtime_process::{RuntimeIdentity, bind_public_socket};
use d2bd_runtime::typed_error::TypedError;
use d2bd_runtime::unix_transport::{read_frame, set_frame_read_deadline, write_json_frame};
use socket2::Socket;

use crate::provider_lifecycle::ProviderRuntime;

/// The refusal code for a forwarded payload this endpoint cannot read as the
/// canonical object the broker validated.
pub(crate) const INVALID_PAYLOAD: &str = "invalid-payload";

/// The read deadline for one forwarded request frame: a connected peer that
/// sends nothing is closed rather than pinning a connection thread.
const FORWARD_REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// The accept-loop poll interval. The listener is nonblocking, so the loop
/// sleeps between drains exactly as the public accept loop does.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The rendezvous socket the environment names, when it names one.
///
/// The variable is declared with the carrier in `d2b-contracts-broker`, so
/// the broker that dials and the daemon that binds cannot disagree on it.
pub(crate) fn configured_socket() -> Option<PathBuf> {
    std::env::var_os(FORWARD_SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The started providers of every Zone, keyed by Zone label.
#[derive(Default)]
pub(crate) struct ForwardRendezvous {
    zones: Mutex<BTreeMap<String, Arc<ProviderRuntime>>>,
}

impl ForwardRendezvous {
    /// Assemble an empty rendezvous: no Zone has started providers yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publish the providers one Zone started.
    ///
    /// A Zone whose plane re-opens republishes its new provider set; the last
    /// published set is the one that answers.
    pub(crate) fn publish(&self, zone: &str, providers: Arc<ProviderRuntime>) {
        self.zones
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(zone.to_owned(), providers);
    }

    /// Answer one forwarded invocation.
    ///
    /// A Zone with no started providers and an operation no started provider
    /// declares are the same refusal: nothing in this process serves the
    /// call, and the code the broker's own envelope uses for that state names
    /// it.
    pub(crate) async fn invoke(
        &self,
        request: &ForwardOperationRequest,
    ) -> ForwardOperationResponse {
        let providers = self
            .zones
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&request.zone)
            .map(Arc::clone);
        let Some(providers) = providers else {
            return refused(UNCOMMITTED_OPERATION);
        };
        let Some(provider) = providers.declaring_provider(&request.operation) else {
            return refused(UNCOMMITTED_OPERATION);
        };
        let Ok(bytes) = serde_json::to_vec(&request.payload) else {
            return refused(INVALID_PAYLOAD);
        };
        let Ok(payload) = CanonicalJsonObject::parse(&bytes) else {
            return refused(INVALID_PAYLOAD);
        };
        match provider
            .invoke(&request.operation, &request.invocation_id, payload)
            .await
        {
            Ok(result) => ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::to_value(result.object())
                        .expect("canonical JSON objects always serialize"),
                },
            },
            Err(failure) => refused(failure.code()),
        }
    }

    /// Answer one connection: read one request frame, invoke, write one
    /// reply frame.
    fn serve_connection(
        &self,
        connection: &Socket,
        runtime: &tokio::runtime::Handle,
    ) -> Result<(), TypedError> {
        set_frame_read_deadline(connection, Some(FORWARD_REQUEST_DEADLINE));
        let frame = read_frame(connection)?;
        let request: ForwardOperationRequest =
            serde_json::from_slice(&frame).map_err(|error| TypedError::WireInvalidFrame {
                detail: format!(
                    "forwarded request frame is not a ForwardOperationRequest: {error}"
                ),
            })?;
        let response = runtime.block_on(self.invoke(&request));
        write_json_frame(connection, &response)
    }
}

/// One refused forwarded invocation, named.
fn refused(code: &str) -> ForwardOperationResponse {
    ForwardOperationResponse {
        outcome: ForwardOperationOutcome::Refused {
            code: code.to_owned(),
        },
    }
}

/// Bind the rendezvous listener at `path`.
///
/// The posture is the public socket's: a seqpacket listener owned by the
/// daemon, mode 0660, chgrp'd to the socket group so the broker's uid reaches
/// it and nothing else does.
pub(crate) fn bind(path: &Path, identity: &RuntimeIdentity) -> Result<Socket, TypedError> {
    bind_public_socket(path, identity)
}

/// Serve accepted forwarded connections until the daemon exits.
pub(crate) fn spawn_server(
    rendezvous: Arc<ForwardRendezvous>,
    listener: Socket,
    runtime: tokio::runtime::Handle,
) -> Result<(), TypedError> {
    std::thread::Builder::new()
        .name("d2b-forward-rendezvous".to_owned())
        .spawn(move || serve_accepted(rendezvous, listener, runtime))
        .map(|_| ())
        .map_err(|error| TypedError::InternalIo {
            context: "spawn forward rendezvous listener".to_owned(),
            detail: error.to_string(),
        })
}

/// The accept loop: one bounded connection thread per accepted call.
fn serve_accepted(
    rendezvous: Arc<ForwardRendezvous>,
    listener: Socket,
    runtime: tokio::runtime::Handle,
) {
    let semaphore = ConnSemaphore::new(DEFAULT_MAX_INFLIGHT_CONNECTIONS);
    loop {
        let connection = match listener.accept() {
            Ok((connection, _)) => connection,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
                continue;
            }
            Err(error) => {
                tracing::warn!(error = %error, "forward rendezvous accept failed; continuing");
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
                continue;
            }
        };
        let Some(permit) = semaphore.try_acquire() else {
            // The cap is the admission gate. A call refused here is closed
            // without an answer, which the broker reports as a missing
            // handler rather than a served call.
            tracing::warn!("forward rendezvous is at its in-flight cap; closing the call");
            continue;
        };
        if connection.set_nonblocking(false).is_err() {
            continue;
        }
        let rendezvous = Arc::clone(&rendezvous);
        let runtime = runtime.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("d2b-forward-conn".to_owned())
            .spawn(move || {
                let _permit = permit;
                if let Err(error) = rendezvous.serve_connection(&connection, &runtime) {
                    tracing::warn!(
                        reason = %error.message(),
                        "forward rendezvous call refused"
                    );
                }
            })
        {
            // The spawn failure drops the closure, and with it the permit and
            // the connection: the slot is released and the call refused.
            tracing::warn!(error = %error, "forward rendezvous call thread refused");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use d2b_contracts_resource::v3::canonical_json_bytes;
    use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
    use d2b_provider_process::{
        ExecutionMode, INVALID_PROCESS_TYPE, ProcessDriverArgs, ProcessDriverEffects,
        ProcessFamilySpec, ProcessResourceIdentity, ProviderAdoption, ProviderLiveness,
        process_family_descriptors,
    };
    use d2bd_runtime::unix_transport::connect_seqpacket;

    use super::*;
    use crate::provider_lifecycle::{ProviderSet, family_declaration};

    /// A port that refuses every effect: the pilot operation answers from the
    /// family's declaration alone, so an effect call would fail this test
    /// loudly instead of passing unnoticed.
    struct RefusingEffects;

    #[async_trait::async_trait]
    impl ProcessDriverEffects for RefusingEffects {
        async fn launch(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            Err("refused".to_owned())
        }

        async fn launch_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            Err("refused".to_owned())
        }

        async fn adopt(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn adopt_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn stop(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_stale(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &AdoptionCandidate,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        async fn device_worker_launch(
            &self,
            _ctx: &mut d2b_resource_runtime::context::ResourceContext,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessFamilySpec,
        ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
            Ok(None)
        }

        async fn finalize(&self, _identity: &ProcessResourceIdentity) -> Result<(), String> {
            Err("refused".to_owned())
        }

        fn has_active(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            false
        }
    }

    /// The socket identity the test binds under: the caller's own uid/gid,
    /// with the production root-owned-parent check off, exactly as the
    /// daemon's unprivileged test mode resolves it.
    fn test_identity() -> RuntimeIdentity {
        RuntimeIdentity {
            daemon_uid: nix::unistd::getuid(),
            daemon_gid: nix::unistd::getgid(),
            public_socket_gid: nix::unistd::getgid(),
            unsafe_local_helper_socket_gid: None,
            expect_root_owned_parent: false,
        }
    }

    /// One started Zone whose `Process` family declares the pilot operation,
    /// served by a rendezvous on a real socket.
    struct ServingRendezvous {
        socket_path: PathBuf,
        _scratch: tempfile::TempDir,
        _providers: Arc<ProviderRuntime>,
    }

    impl ServingRendezvous {
        async fn start() -> Self {
            let zone = ZoneId::parse("test").expect("the test zone label is canonical");
            let scratch = tempfile::tempdir().expect("test scratch");
            let providers = ProviderSet::new(zone.clone(), scratch.path().to_path_buf())
                .with(
                    family_declaration("process"),
                    Vec::from(process_family_descriptors(ProcessDriverArgs {
                        zone: zone.clone(),
                        effects: Arc::new(RefusingEffects),
                        zone_uid: None,
                        policy_revision: None,
                        provider_assignment_generation: None,
                        controller_generation: ControllerGeneration::new(1)
                            .expect("the test generation is canonical"),
                        guest_execution: None,
                        mode: ExecutionMode::Host,
                    })),
                )
                .start()
                .await
                .expect("the process family starts through the base");
            let providers = Arc::new(providers);
            let rendezvous = Arc::new(ForwardRendezvous::new());
            rendezvous.publish(zone.as_str(), Arc::clone(&providers));
            let socket_path = scratch.path().join("d2bd-forward.sock");
            let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
            spawn_server(
                Arc::clone(&rendezvous),
                listener,
                tokio::runtime::Handle::current(),
            )
            .expect("spawn the rendezvous server");
            Self {
                socket_path,
                _scratch: scratch,
                _providers: providers,
            }
        }
    }

    /// Forward one invocation over a real socket the way the broker's
    /// forwarder does: one connection, one canonically encoded request frame,
    /// one reply frame.
    fn forward(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> ForwardOperationResponse {
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        let request = ForwardOperationRequest {
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
        };
        // The broker encodes the request with the canonical profile, so the
        // endpoint is exercised against the exact bytes the broker sends.
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        d2bd_runtime::unix_transport::write_frame(&socket, &encoded)
            .expect("write the request frame");
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// A forwarded call crosses a real socket and the declared handler
    /// answers it: the result carries the family's own declaration, the Zone,
    /// and the invocation identifier the caller forwarded. A carrier that
    /// never reached the handler could not produce these values.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_forwarded_call_crosses_the_socket_and_the_declared_handler_answers() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        );
        let ForwardOperationOutcome::Result { result } = response.outcome else {
            panic!("the declared operation must answer, got a refusal");
        };
        assert_eq!(result["family"], "process");
        assert_eq!(result["resourceType"], "Process");
        assert_eq!(
            result["memberTypes"],
            serde_json::json!(["Process", "EphemeralProcess"])
        );
        assert_eq!(result["zone"], "test");
        assert_eq!(
            result["operations"],
            serde_json::json!(["inspect-process-family"])
        );
        assert_eq!(result["verbs"][0], "get");
        assert_eq!(result["execution"], serde_json::json!(["host", "guest"]));
        // The handler's context carries the identifier the broker minted
        // before it forwarded, so both records name one invocation.
        assert_eq!(result["invocation"], "invocation-7");
    }

    /// An operation no started provider declares is refused by name, and so is
    /// a call naming a Zone this process has no providers for.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_undeclared_operation_is_refused_by_name() {
        let serving = ServingRendezvous::start().await;
        for (operation, zone) in [
            ("NoSuchOperation", "test"),
            ("inspect-process-family", "no-such-zone"),
        ] {
            let response = forward(
                &serving.socket_path,
                operation,
                zone,
                serde_json::json!({ "resourceType": "Process" }),
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: UNCOMMITTED_OPERATION.to_owned(),
                },
                "{operation} in {zone} is not served by this process"
            );
        }
    }

    /// A refusal the handler itself decided crosses the socket under its own
    /// code, so the peer's record and the passed-through detail keep the
    /// family's vocabulary rather than a carrier-level one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_handler_refusal_crosses_back_under_its_own_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Quota" }),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: INVALID_PROCESS_TYPE.to_owned(),
            }
        );
    }
}
