//! The broker's exact-endpoint ACL wire (U18, R23).
//!
//! A consumer that is admitted one exact host endpoint must get that endpoint
//! and nothing else. Three things could widen it, and each is closed here
//! before any path is touched:
//!
//! 1. **A path on the wire.** There is none. The request carries a
//!    [`BoundedToken`] socket NAME, and a name is joined onto a directory this
//!    module derives from the broker's own serve-time configuration - the
//!    runtime root the private broker socket's parent already names, the same
//!    fence [`crate::live_handlers::grant_serving_worker_launch_acls`] and
//!    [`crate::ops::device_worker`] apply. The token's `^[a-z][a-z0-9-]*$`
//!    grammar admits no `/`, no `.`, and no `..`, so an alternate absolute
//!    socket, a `..` escape, and the containing directory are values the wire
//!    cannot represent rather than refusals this code has to detect. The
//!    resolved path is then re-checked as a direct child of that directory, so
//!    the property does not rest on the grammar alone.
//! 2. **A principal a caller chose.** The uid the ACL names is derived by
//!    [`crate::ops::consumer_principal::resolve_consumer_principal`] from the
//!    verified Zone bundle, from the committed consumer reference alone. A
//!    request that also carries a claim has it re-pinned and refused on
//!    disagreement, and the returned principal is the derived one either way.
//! 3. **A relationship repointed at another endpoint.** The authority binding
//!    is recomputed here from the request's own committed facts and compared
//!    BEFORE the path is resolved, exactly as
//!    [`crate::ops::security_key::validate_device_authority`] compares the
//!    security-key binding. It is a consistency proof and never a secret; the
//!    grant itself still lands only on the inode this module resolved and
//!    pinned.
//!
//! The host effect is the trio of exact-endpoint helpers in
//! [`crate::live_handlers`], and every answer reports the KERNEL's effective
//! rights on the pinned inode rather than the mode the broker asked for.
//!
//! The directory those names resolve inside is the broker's own, and the
//! broker creates it: [`ensure_endpoint_socket_dir`] provisions it once at
//! serve time, before any connection is accepted, at a mode whose group class
//! keeps a grant's traverse entry effective. Nothing on the request path
//! creates it, and an absent directory is still refused by name - a request
//! never invents the tree its own effect would run against.
//!
//! Public arm: `tests/endpoint_delivery.rs` drives this resolution from
//! outside the crate, for the same reason the arms above it are public - the
//! integration test is what proves the boundary, and it can only reach the
//! boundary the dispatch arm itself calls.

use std::path::{Path, PathBuf};

use d2b_contracts_broker::broker_wire::{
    BrokerRequest, EndpointAccessRequest, EndpointAccessResponse, EndpointAccessVerb,
    EndpointPrincipalClaim, endpoint_access_authority_binding,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_core::bundle_resolver::BundleResolver;

use super::consumer_principal::{repin_consumer_principal, resolve_consumer_principal};

/// The directory under the broker runtime root that holds the endpoint
/// sockets the broker mediates.
///
/// One broker-owned directory, named by the broker and never by a caller, is
/// what turns a socket NAME into a path. A caller that names a sibling socket
/// in this directory still gets a grant on that sibling's own inode and
/// nothing beyond it; a caller that wants a different directory has no
/// vocabulary for saying so.
pub const ENDPOINT_SOCKET_DIR: &str = "endpoints";

/// The most bytes an authority key may occupy.
///
/// The key is a fixed-width digest, so anything longer is not one, and the
/// comparison below is a constant-time-shape equality over a bounded value.
const MAX_AUTHORITY_KEY_BYTES: usize = 128;

/// The highest POSIX permission a single endpoint grant may ask for.
///
/// A Unix-domain socket is reached, not listed, and the exact-endpoint
/// contract withholds directory authority, so the ceiling is the socket's own
/// read/write/traverse triple. Zero asks for nothing and is refused rather
/// than recorded as a grant.
const MAX_SOCKET_RIGHTS: u8 = 0o7;

/// The closed reason an exact-endpoint access request was refused.
///
/// Every variant names the condition, never the material: a refusal slug
/// carries no host path, no socket name, and no number. The structured
/// fields stay inside the broker, where the audit record can name the
/// committed fact the verified bundle answered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointAccessError {
    /// The request is not one of the three exact-endpoint variants.
    NotAnEndpointRequest,
    /// The endpoint reference is not an `Endpoint` row, or the authority key
    /// is absent or is not the fixed-width digest it must be.
    EndpointAuthorityRefused,
    /// The recomputed binding does not reproduce the request's key, so the
    /// request does not describe the relationship its key was minted for.
    EndpointAuthorityMismatch,
    /// The requested permission is zero or wider than a socket's own triple.
    SocketRightsOutOfRange,
    /// The broker's runtime root is not an absolute, normalized directory, so
    /// nothing under it can be resolved safely.
    RuntimeRootInvalid,
    /// The socket name does not resolve to a direct child of the broker's own
    /// endpoint directory. The token grammar makes this unreachable; the
    /// check is here so the property is structural rather than inherited.
    NotADirectChild,
    /// The broker's own endpoint directory is absent, so the effect has
    /// nothing to act on. The broker does not create it here: it is created
    /// once at serve time by [`ensure_endpoint_socket_dir`], and the endpoint
    /// owner prepares the endpoint inside it, because a request never invents
    /// the tree the effect runs against.
    EndpointDirectoryAbsent,
    /// The broker's own runtime root cannot host the endpoint directory it
    /// owns, so this surface has nowhere to land. A serve-time failure, never
    /// a request-time one: the broker creates that directory before it accepts
    /// a connection, so a broker that could not does not serve it at all.
    RuntimeRootUnusable { detail: String },
    /// No exact endpoint resolves at the broker's resolved path.
    EndpointAbsent,
    /// The consumer principal could not be derived, or a claimed principal
    /// did not reproduce the derivation. The slug is the closed code
    /// [`super::consumer_principal`] renders.
    ConsumerPrincipal { code: String },
    /// The pinned ACL effect itself failed. The detail is the helper's own
    /// refusal, which is where the kernel-level reason is reported.
    Effect { detail: String },
}

impl EndpointAccessError {
    /// The closed, path-free slug a refusal and its audit record carry.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAnEndpointRequest => "endpoint-access-wrong-variant",
            Self::EndpointAuthorityRefused => "endpoint-access-authority-required",
            Self::EndpointAuthorityMismatch => "endpoint-access-authority-mismatch",
            Self::SocketRightsOutOfRange => "endpoint-access-rights-out-of-range",
            Self::RuntimeRootInvalid => "endpoint-access-runtime-root-invalid",
            Self::NotADirectChild => "endpoint-access-not-a-direct-child",
            Self::EndpointDirectoryAbsent => "endpoint-access-directory-absent",
            Self::RuntimeRootUnusable { .. } => "endpoint-access-runtime-root-unwritable",
            Self::EndpointAbsent => "endpoint-access-endpoint-absent",
            Self::ConsumerPrincipal { .. } => "endpoint-access-consumer-principal",
            Self::Effect { .. } => "endpoint-access-effect-failed",
        }
    }
}

impl std::fmt::Display for EndpointAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for EndpointAccessError {}

/// The broker-owned directory the exact endpoint sockets live in.
///
/// Derived from the broker runtime root and nothing else: the root is the
/// parent of the private broker socket, which the daemon derives a runner's
/// own socket paths from and which no request can name.
pub fn endpoint_socket_directory(runtime_root: &Path) -> Result<PathBuf, EndpointAccessError> {
    if !crate::live_handlers::is_anchored_absolute(runtime_root) || runtime_root.parent().is_none()
    {
        return Err(EndpointAccessError::RuntimeRootInvalid);
    }
    let directory = runtime_root.join(ENDPOINT_SOCKET_DIR);
    if !directory.starts_with(runtime_root) || directory == *runtime_root {
        return Err(EndpointAccessError::RuntimeRootInvalid);
    }
    Ok(directory)
}

/// The mode the broker's own endpoint directory is created with.
///
/// `0710`: the owner may create, list and remove; everyone else gets the
/// traverse bit and nothing more, so the directory is never listable by a
/// principal the broker admits - listing it is the directory authority R23
/// withdrew, and the traversal grant a verb installs on this ancestor
/// deliberately carries no read bit.
///
/// The group class is load-bearing rather than incidental. POSIX rewrites an
/// ACL mask from a file's GROUP bits on every `chmod`, so a mode with no
/// group bits at all (`0700`) nullifies the named traverse entry
/// [`crate::live_handlers::grant_exact_endpoint_access`] installs here the
/// moment anything re-asserts the mode - the recorded failure in
/// `docs/solutions/infrastructure/posix-acl-mask-nullified-by-chmod-on-mode-0700-directories.md`.
/// A group bit of exactly `x` leaves the mask at `mask::--x`, so the named
/// entry stays effective and no other principal gains anything.
const ENDPOINT_SOCKET_DIR_MODE: u32 = 0o710;

/// The broker's own endpoint directory, created once at serve time.
///
/// The broker owns its runtime root - it is the private socket's parent, and
/// the per-Guest and per-endpoint socket trees below it are already the
/// broker's to provision - so it owns the one directory this surface resolves
/// socket names inside. Creating it here rather than inside the grant path is
/// the point: a request never invents the tree its own effect runs against,
/// and a lazily created leaf would be a request deciding the posture the ACL
/// then lands on.
///
/// [`accept_endpoint_access`] still refuses an absent directory by name. This
/// is the only thing that creates the leaf, it runs once before any
/// connection is accepted, and a broker that could not create it does not
/// serve a surface that would refuse every verb for a reason of its own
/// making.
///
/// An existing directory keeps its own mode: re-asserting one would rewrite
/// the ACL mask over whatever a live grant has already installed, which is
/// the trap the recorded solution above is about.
pub fn ensure_endpoint_socket_dir(
    runtime_root: &Path,
) -> Result<PathBuf, EndpointAccessError> {
    let directory = endpoint_socket_directory(runtime_root)?;
    crate::sys::path_safe::ensure_dir_preserve_existing(
        &directory,
        ENDPOINT_SOCKET_DIR_MODE,
        None,
        None,
    )
    .map_err(|error| EndpointAccessError::RuntimeRootUnusable {
        detail: format!("create {}: {error}", directory.display()),
    })?;
    Ok(directory)
}

/// The one filesystem object one exact-endpoint request can select.
///
/// The socket token is a single path component by grammar, and the result is
/// re-checked to be a direct child of the broker's own endpoint directory, so
/// this function's return type is the whole reachable set: there is no input
/// for which it names a sibling directory, an ancestor, or a foreign tree.
pub fn endpoint_socket_path(
    runtime_root: &Path,
    socket: &BoundedToken,
) -> Result<PathBuf, EndpointAccessError> {
    let directory = endpoint_socket_directory(runtime_root)?;
    let path = directory.join(socket.as_str());
    if path.parent() != Some(directory.as_path()) {
        return Err(EndpointAccessError::NotADirectChild);
    }
    Ok(path)
}

/// The directories between the broker runtime root and the socket, root-first.
///
/// Bounded to the broker's own tree on purpose: the ancestors above the
/// runtime root belong to the host, and a traversal grant the broker installs
/// outside its own runtime root is a grant it does not own. An ancestor that
/// is already world-traversable needs no grant and is skipped by the helper
/// anyway, so including the runtime root itself is both necessary and inert
/// in the common case.
fn traversal_ancestors(runtime_root: &Path, socket: &Path) -> Vec<PathBuf> {
    let mut ancestors = Vec::new();
    let Some(parent) = socket.parent() else {
        return ancestors;
    };
    for directory in parent.ancestors() {
        if !directory.starts_with(runtime_root) {
            break;
        }
        ancestors.push(directory.to_path_buf());
    }
    ancestors.reverse();
    ancestors
}

/// The consumer principal one request acts for.
///
/// The derivation reads the verified Zone bundle and the committed consumer
/// reference, and nothing else. A claim the request carries is re-pinned
/// against that derivation and refused on disagreement, so the number the ACL
/// is applied to is the broker's own answer whether or not the caller agreed
/// with it.
fn consumer_principal(
    resolver: &BundleResolver,
    request: &EndpointAccessRequest,
) -> Result<d2b_core::bundle_resolver::ConsumerPrincipal, EndpointAccessError> {
    let derive = || resolve_consumer_principal(resolver, &request.zone_uid, &request.consumer_ref);
    let principal = match request.claimed_principal {
        None => derive(),
        Some(EndpointPrincipalClaim { uid, gid }) => repin_consumer_principal(
            resolver,
            &request.zone_uid,
            &request.consumer_ref,
            &d2b_core::bundle_resolver::ConsumerPrincipal { uid, gid },
        ),
    };
    principal.map_err(|error| EndpointAccessError::ConsumerPrincipal {
        code: error.to_string(),
    })
}

/// Require the request's own admission proof before anything is resolved.
///
/// Three checks, in this order, all of them on values the request carries and
/// none of them touching the filesystem:
///
/// 1. the endpoint is an `Endpoint` row and the key is present and the size a
///    digest has;
/// 2. the recomputed binding reproduces the key, which pins the socket name,
///    the consumer, the Zone, and the VERB - so a grant's key cannot be
///    replayed as an observation or a revoke, and a relationship cannot be
///    repointed at a different socket by editing one field;
/// 3. the requested permission is a real socket permission.
fn require_authority(
    request: &EndpointAccessRequest,
    verb: EndpointAccessVerb,
) -> Result<(), EndpointAccessError> {
    if request.endpoint_ref.resource_type().as_str() != "Endpoint"
        || request.authority_key.is_empty()
        || request.authority_key.len() > MAX_AUTHORITY_KEY_BYTES
    {
        return Err(EndpointAccessError::EndpointAuthorityRefused);
    }
    if request.authority_key
        != endpoint_access_authority_binding(
            &request.endpoint_ref,
            &request.consumer_ref,
            &request.zone_uid,
            &request.socket,
            verb,
        )
    {
        return Err(EndpointAccessError::EndpointAuthorityMismatch);
    }
    if request.socket_rights == 0 || request.socket_rights > MAX_SOCKET_RIGHTS {
        return Err(EndpointAccessError::SocketRightsOutOfRange);
    }
    Ok(())
}

/// Serve one exact-endpoint access request off the broker wire.
///
/// This is what the dispatch arms call, and it is the whole resolution path:
/// the verb comes from the variant, the request is one shared struct, and the
/// broker's runtime root plus the verified Zone bundle are the only inputs the
/// effect derives from. `runtime_root` is the broker's own serve-time
/// configuration (the private socket's parent); it is a parameter rather than
/// a lookup so the dispatch arm and its test drive the identical path.
pub fn accept_endpoint_access(
    request: &BrokerRequest,
    runtime_root: &Path,
    resolver: &BundleResolver,
) -> Result<EndpointAccessResponse, EndpointAccessError> {
    let (verb, request) = match request {
        BrokerRequest::EndpointObserve(request) => (EndpointAccessVerb::Observe, request),
        BrokerRequest::EndpointGrantAccess(request) => (EndpointAccessVerb::Grant, request),
        BrokerRequest::EndpointRevokeAccess(request) => (EndpointAccessVerb::Revoke, request),
        _ => return Err(EndpointAccessError::NotAnEndpointRequest),
    };
    // The admission proof first: nothing below this line is reached by a
    // request that does not reproduce its own binding.
    require_authority(request, verb)?;
    let directory = endpoint_socket_directory(runtime_root)?;
    if !directory.is_dir() {
        return Err(EndpointAccessError::EndpointDirectoryAbsent);
    }
    let socket = endpoint_socket_path(runtime_root, &request.socket)?;
    let principal = consumer_principal(resolver, request)?;

    // Every verb answers the same closed code when the broker's own resolved
    // path holds no socket at all, so a caller learns "this endpoint is not
    // prepared" instead of having to read an effect failure to find out. The
    // read is the observation the observe verb returns, so nothing here is
    // spent twice.
    let present = crate::live_handlers::exact_endpoint_access(&socket, principal.uid)
        .map_err(effect_error)?
        .ok_or(EndpointAccessError::EndpointAbsent)?;

    // The answer is always the KERNEL's answer over the WHOLE path, for every
    // verb. The grant helper reports traversal only for the ancestor list it
    // was handed - the chain inside the broker's own runtime root, which is
    // the only chain the broker owns and may mutate - so taking its traversal
    // figure for the reply would answer "the consumer can walk to the socket"
    // from a check that never looked above the broker's own root. That is the
    // AE19 trap this crate already records: a correct grant one level below a
    // root with no traverse is no grant at all. The reply therefore re-reads
    // the full chain, which is also why the field means the same thing after
    // an observe, a grant, and a revoke.
    let (pinned, answered) = match verb {
        EndpointAccessVerb::Observe => (present.socket(), present),
        EndpointAccessVerb::Grant => {
            let granted = crate::live_handlers::grant_exact_endpoint_access(
                &socket,
                &traversal_ancestors(runtime_root, &socket),
                principal.uid,
                u32::from(request.socket_rights),
            )
            .map_err(effect_error)?;
            let answered = full_path_access(&socket, principal.uid)?;
            (granted.socket(), answered)
        }
        EndpointAccessVerb::Revoke => {
            // The removed entry is reported against the inode the removal
            // landed on, so a retry can tell a removal that took effect from
            // one that found nothing - and a producer that replaced its
            // socket in between shows up as a different inode here rather
            // than as a silent success.
            let removed =
                crate::live_handlers::revoke_exact_endpoint_access(&socket, principal.uid)
                    .map_err(effect_error)?
                    .ok_or(EndpointAccessError::EndpointAbsent)?;
            (removed, full_path_access(&socket, principal.uid)?)
        }
    };
    Ok(EndpointAccessResponse {
        endpoint_ref: request.endpoint_ref.clone(),
        consumer_ref: request.consumer_ref.clone(),
        socket: request.socket.clone(),
        socket_device: pinned.0,
        socket_inode: pinned.1,
        socket_effective_rights: answered.socket_effective(),
        ancestors_traversable: answered.ancestor_effective() & 0o1 == 0o1,
        parent_listable: answered.parent_listable(),
        consumer_uid: principal.uid,
        consumer_gid: principal.gid,
    })
}

/// The kernel's answer for one principal over the whole path to the socket.
///
/// Re-read rather than inferred from an effect: the ancestor traversal the
/// grant installed outlives the entry it installed on the socket, and only a
/// fresh read says whether the consumer can actually walk there.
fn full_path_access(
    socket: &Path,
    uid: u32,
) -> Result<crate::live_handlers::EndpointAccess, EndpointAccessError> {
    crate::live_handlers::exact_endpoint_access(socket, uid)
        .map_err(effect_error)?
        .ok_or(EndpointAccessError::EndpointAbsent)
}

/// The one place a live-handler refusal becomes this arm's refusal, so the
/// error's shape cannot drift between the three verbs.
fn effect_error(detail: String) -> EndpointAccessError {
    EndpointAccessError::Effect { detail }
}

/// The audit target one exact-endpoint request records against.
///
/// The committed endpoint reference: an opaque identity that joins a record
/// to the relationship it describes, with no socket name and no host path in
/// it.
pub fn audit_target(request: &EndpointAccessRequest) -> String {
    request.endpoint_ref.to_canonical_string()
}
