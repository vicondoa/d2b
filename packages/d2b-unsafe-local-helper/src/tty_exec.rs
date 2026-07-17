//! One-shot terminal launch behind the authenticated systemd user runtime.
//!
//! ComponentSession admission, deadlines, and idempotency are resolved before
//! this adapter receives a [`RuntimeOwner`] and terminal attachment. The adapter
//! owns descriptor, policy, transient-scope, cancellation, and teardown checks;
//! it has no inherited-descriptor or host-shell execution path.

use crate::environment::ManagerEnvironment;
use crate::services::runtime_systemd_user::{
    AuthenticatedTerminalAttachment, RuntimeOwner, TERMINAL_ATTACHMENT_INDEX,
};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{FileType, OFlags, fcntl_getfl, fstat};
use rustix::io::{FdFlags, fcntl_getfd};
use rustix::termios::tcgetattr;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

const EXIT_RUNTIME_SERVICE_REQUIRED: i32 = 69;
const MAX_ARGV_ENTRIES: usize = 64;
const MAX_ARGV_BYTES: usize = 64 * 1024;
const MAX_ACTIVE_REQUESTS: usize = 64;
const MAX_TERMINAL_REQUESTS: usize = 128;
const MAX_ID_BYTES: usize = 64;

#[derive(Clone, PartialEq, Eq)]
pub struct TtyOneShotSpec {
    rows: u16,
    cols: u16,
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl TtyOneShotSpec {
    pub fn new(
        owner: &RuntimeOwner,
        rows: u16,
        cols: u16,
        argv: Vec<String>,
        raw_environment: Vec<String>,
    ) -> Result<Self, TtyOneShotError> {
        let encoded_bytes = argv.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(value.len())
                .and_then(|size| size.checked_add(1))
        });
        if rows == 0
            || cols == 0
            || argv.is_empty()
            || argv.len() > MAX_ARGV_ENTRIES
            || encoded_bytes.is_none_or(|size| size > MAX_ARGV_BYTES)
            || !argv[0].starts_with('/')
            || argv
                .iter()
                .any(|value| value.is_empty() || value.contains('\0'))
        {
            return Err(TtyOneShotError::InvalidPolicy);
        }
        let environment =
            ManagerEnvironment::parse_for_authenticated_uid(raw_environment, owner.uid())
                .and_then(|manager| manager.child_entries(false, None))
                .map_err(|_| TtyOneShotError::InvalidPolicy)?;
        Ok(Self {
            rows,
            cols,
            argv,
            environment,
        })
    }

    pub const fn rows(&self) -> u16 {
        self.rows
    }

    pub const fn cols(&self) -> u16 {
        self.cols
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

impl fmt::Debug for TtyOneShotSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TtyOneShotSpec")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("argv_count", &self.argv.len())
            .field("environment_count", &self.environment.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TtyOneShotRequest {
    request_id: [u8; 16],
    session_generation: u64,
    resource_id: String,
    operation_id: String,
}

impl TtyOneShotRequest {
    pub fn new(
        request_id: [u8; 16],
        session_generation: u64,
        resource_id: String,
        operation_id: String,
    ) -> Result<Self, TtyOneShotError> {
        if request_id == [0; 16]
            || session_generation == 0
            || !valid_id(&resource_id)
            || !valid_id(&operation_id)
        {
            return Err(TtyOneShotError::InvalidRequest);
        }
        Ok(Self {
            request_id,
            session_generation,
            resource_id,
            operation_id,
        })
    }

    pub const fn request_id(&self) -> [u8; 16] {
        self.request_id
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

impl fmt::Debug for TtyOneShotRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TtyOneShotRequest(<redacted>)")
    }
}

pub struct ValidatedTerminal {
    fd: OwnedFd,
}

impl ValidatedTerminal {
    pub fn new(
        owner: &RuntimeOwner,
        request: &TtyOneShotRequest,
        attachment: &AuthenticatedTerminalAttachment,
        fd: OwnedFd,
    ) -> Result<Self, TtyOneShotError> {
        if request.session_generation != owner.session_generation()
            || attachment.index != TERMINAL_ATTACHMENT_INDEX
            || attachment.owner_uid != owner.uid()
            || attachment.session_generation != owner.session_generation()
            || attachment.request_id != request.request_id
            || !attachment.connected_stream
            || !attachment.cloexec
        {
            return Err(TtyOneShotError::AttachmentMismatch);
        }
        let flags = fcntl_getfd(&fd).map_err(|_| TtyOneShotError::AttachmentMismatch)?;
        let status = fcntl_getfl(&fd).map_err(|_| TtyOneShotError::AttachmentMismatch)?;
        let metadata = fstat(&fd).map_err(|_| TtyOneShotError::AttachmentMismatch)?;
        if !flags.contains(FdFlags::CLOEXEC)
            || status & OFlags::ACCMODE != OFlags::RDWR
            || FileType::from_raw_mode(metadata.st_mode) != FileType::CharacterDevice
            || tcgetattr(&fd).is_err()
        {
            return Err(TtyOneShotError::AttachmentMismatch);
        }
        Ok(Self { fd })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl fmt::Debug for ValidatedTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidatedTerminal(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TransientUserScope {
    request_id: [u8; 16],
    owner_uid: u32,
    session_generation: u64,
    scope_name: String,
    invocation_id: String,
    cgroup_leaf: String,
}

impl TransientUserScope {
    pub fn new(
        owner: &RuntimeOwner,
        request_id: [u8; 16],
        scope_name: String,
        invocation_id: String,
        cgroup_leaf: String,
    ) -> Result<Self, TtyOneShotError> {
        let expected_scope = format!("d2b-tty-{}.scope", hex_request_id(request_id));
        let expected_leaf = format!(
            "/user.slice/user-{}.slice/user@{}.service/app.slice/{expected_scope}",
            owner.uid(),
            owner.uid()
        );
        if request_id == [0; 16]
            || scope_name != expected_scope
            || cgroup_leaf != expected_leaf
            || invocation_id.len() != 32
            || !invocation_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(TtyOneShotError::ScopeOwnershipMismatch);
        }
        Ok(Self {
            request_id,
            owner_uid: owner.uid(),
            session_generation: owner.session_generation(),
            scope_name,
            invocation_id,
            cgroup_leaf,
        })
    }

    fn belongs_to(&self, owner: &RuntimeOwner, request_id: [u8; 16]) -> bool {
        self.request_id == request_id
            && self.owner_uid == owner.uid()
            && self.session_generation == owner.session_generation()
            && self.scope_name == format!("d2b-tty-{}.scope", hex_request_id(request_id))
            && self.cgroup_leaf
                == format!(
                    "/user.slice/user-{}.slice/user@{}.service/app.slice/{}",
                    owner.uid(),
                    owner.uid(),
                    self.scope_name
                )
            && self.invocation_id.len() == 32
            && self
                .invocation_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }
}

impl fmt::Debug for TransientUserScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransientUserScope(<redacted>)")
    }
}

pub trait TtyOneShotRuntime {
    fn start_transient_user_scope(
        &mut self,
        owner: &RuntimeOwner,
        request: &TtyOneShotRequest,
        spec: &TtyOneShotSpec,
        terminal: ValidatedTerminal,
    ) -> Result<TransientUserScope, TtyOneShotError>;

    fn teardown_transient_user_scope(
        &mut self,
        owner: &RuntimeOwner,
        scope: &TransientUserScope,
    ) -> Result<(), TtyOneShotError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    TeardownComplete,
    AlreadyTerminal,
    UnknownRequest,
}

pub struct TtyOneShotService<B: TtyOneShotRuntime> {
    owner: RuntimeOwner,
    backend: B,
    active: BTreeMap<[u8; 16], TransientUserScope>,
    terminal: VecDeque<[u8; 16]>,
}

impl<B: TtyOneShotRuntime> TtyOneShotService<B> {
    pub fn new(owner: RuntimeOwner, backend: B) -> Self {
        Self {
            owner,
            backend,
            active: BTreeMap::new(),
            terminal: VecDeque::with_capacity(MAX_TERMINAL_REQUESTS),
        }
    }

    pub fn start(
        &mut self,
        request: &TtyOneShotRequest,
        spec: &TtyOneShotSpec,
        attachment: &AuthenticatedTerminalAttachment,
        fd: OwnedFd,
    ) -> Result<TransientUserScope, TtyOneShotError> {
        if request.session_generation != self.owner.session_generation() {
            return Err(TtyOneShotError::OwnerMismatch);
        }
        if self.active.len() == MAX_ACTIVE_REQUESTS {
            return Err(TtyOneShotError::CapacityExceeded);
        }
        if self.active.contains_key(&request.request_id)
            || self.terminal.contains(&request.request_id)
        {
            return Err(TtyOneShotError::RequestConflict);
        }
        let terminal = ValidatedTerminal::new(&self.owner, request, attachment, fd)?;
        let scope =
            self.backend
                .start_transient_user_scope(&self.owner, request, spec, terminal)?;
        if !scope.belongs_to(&self.owner, request.request_id) {
            let _ = self
                .backend
                .teardown_transient_user_scope(&self.owner, &scope);
            return Err(TtyOneShotError::ScopeOwnershipMismatch);
        }
        self.active.insert(request.request_id, scope.clone());
        Ok(scope)
    }

    pub fn cancel(
        &mut self,
        session_generation: u64,
        request_id: [u8; 16],
    ) -> Result<CancelOutcome, TtyOneShotError> {
        if session_generation != self.owner.session_generation() || request_id == [0; 16] {
            return Err(TtyOneShotError::OwnerMismatch);
        }
        if let Some(scope) = self.active.get(&request_id) {
            self.backend
                .teardown_transient_user_scope(&self.owner, scope)?;
            self.active.remove(&request_id);
            self.remember_terminal(request_id);
            return Ok(CancelOutcome::TeardownComplete);
        }
        Ok(if self.terminal.contains(&request_id) {
            CancelOutcome::AlreadyTerminal
        } else {
            CancelOutcome::UnknownRequest
        })
    }

    pub fn teardown_all(&mut self) -> Result<(), TtyOneShotError> {
        let request_ids = self.active.keys().copied().collect::<Vec<_>>();
        let mut failed = false;
        for request_id in request_ids {
            let Some(scope) = self.active.get(&request_id) else {
                continue;
            };
            if self
                .backend
                .teardown_transient_user_scope(&self.owner, scope)
                .is_ok()
            {
                self.active.remove(&request_id);
                self.remember_terminal(request_id);
            } else {
                failed = true;
            }
        }
        if failed {
            Err(TtyOneShotError::TeardownFailed)
        } else {
            Ok(())
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    fn remember_terminal(&mut self, request_id: [u8; 16]) {
        if self.terminal.len() == MAX_TERMINAL_REQUESTS {
            self.terminal.pop_front();
        }
        self.terminal.push_back(request_id);
    }
}

impl<B: TtyOneShotRuntime> Drop for TtyOneShotService<B> {
    fn drop(&mut self) {
        let _ = self.teardown_all();
    }
}

impl<B: TtyOneShotRuntime> fmt::Debug for TtyOneShotService<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TtyOneShotService")
            .field("owner", &"<redacted>")
            .field("active_count", &self.active.len())
            .field("terminal_count", &self.terminal.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtyOneShotError {
    InvalidPolicy,
    InvalidRequest,
    OwnerMismatch,
    AttachmentMismatch,
    ScopeOwnershipMismatch,
    CapacityExceeded,
    RequestConflict,
    RuntimeUnavailable,
    TeardownFailed,
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn hex_request_id(request_id: [u8; 16]) -> String {
    request_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn run(_args: &[String]) -> i32 {
    EXIT_RUNTIME_SERVICE_REQUIRED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::runtime_systemd_user::EstablishedComponentSession;
    use std::fs::OpenOptions;

    struct Session {
        uid: u32,
    }

    impl EstablishedComponentSession for Session {
        fn service_package(&self) -> &str {
            crate::services::runtime_systemd_user::SERVICE_PACKAGE
        }

        fn endpoint_purpose(&self) -> &str {
            crate::services::runtime_systemd_user::ENDPOINT_PURPOSE
        }

        fn endpoint_role(&self) -> &str {
            crate::services::runtime_systemd_user::ENDPOINT_ROLE
        }

        fn is_authenticated(&self) -> bool {
            true
        }

        fn uses_pre_authorized_transport(&self) -> bool {
            true
        }

        fn authenticated_uid(&self) -> u32 {
            self.uid
        }

        fn process_uid(&self) -> u32 {
            self.uid
        }

        fn session_generation(&self) -> u64 {
            7
        }

        fn realm_id(&self) -> &str {
            "local-root"
        }

        fn workload_id(&self) -> &str {
            "terminal"
        }
    }

    #[derive(Default)]
    struct Backend {
        starts: usize,
        teardowns: usize,
        fail_teardown: bool,
        wrong_owner: bool,
    }

    impl TtyOneShotRuntime for Backend {
        fn start_transient_user_scope(
            &mut self,
            owner: &RuntimeOwner,
            request: &TtyOneShotRequest,
            spec: &TtyOneShotSpec,
            terminal: ValidatedTerminal,
        ) -> Result<TransientUserScope, TtyOneShotError> {
            tcgetattr(terminal.as_fd()).map_err(|_| TtyOneShotError::RuntimeUnavailable)?;
            assert_eq!(spec.argv()[0], "/bin/sh");
            assert_eq!(
                spec.environment().get("TERM").map(String::as_str),
                Some("xterm")
            );
            assert_eq!(request.resource_id(), "terminal");
            assert!(request.operation_id().starts_with("open-"));
            self.starts += 1;
            let request_id = if self.wrong_owner {
                [9; 16]
            } else {
                request.request_id()
            };
            let scope_name = format!("d2b-tty-{}.scope", hex_request_id(request_id));
            let cgroup_leaf = format!(
                "/user.slice/user-{}.slice/user@{}.service/app.slice/{scope_name}",
                owner.uid(),
                owner.uid()
            );
            TransientUserScope::new(
                owner,
                request_id,
                scope_name,
                "00112233445566778899aabbccddeeff".to_owned(),
                cgroup_leaf,
            )
        }

        fn teardown_transient_user_scope(
            &mut self,
            _: &RuntimeOwner,
            _: &TransientUserScope,
        ) -> Result<(), TtyOneShotError> {
            self.teardowns += 1;
            if self.fail_teardown {
                Err(TtyOneShotError::TeardownFailed)
            } else {
                Ok(())
            }
        }
    }

    fn owner() -> RuntimeOwner {
        let uid = nix::unistd::getuid().as_raw();
        RuntimeOwner::admit(&Session { uid }).expect("authenticated non-root test owner")
    }

    fn request(id: u8) -> TtyOneShotRequest {
        TtyOneShotRequest::new([id; 16], 7, "terminal".to_owned(), format!("open-{id}")).unwrap()
    }

    fn spec(owner: &RuntimeOwner) -> TtyOneShotSpec {
        TtyOneShotSpec::new(
            owner,
            24,
            80,
            vec!["/bin/sh".to_owned(), "-l".to_owned()],
            vec!["PATH=/bin".to_owned(), "TERM=xterm".to_owned()],
        )
        .unwrap()
    }

    fn terminal_attachment(
        owner: &RuntimeOwner,
        request: &TtyOneShotRequest,
    ) -> AuthenticatedTerminalAttachment {
        AuthenticatedTerminalAttachment {
            index: TERMINAL_ATTACHMENT_INDEX,
            owner_uid: owner.uid(),
            session_generation: owner.session_generation(),
            request_id: request.request_id(),
            connected_stream: true,
            cloexec: true,
        }
    }

    fn terminal_fd() -> OwnedFd {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")
            .expect("open PTY multiplexer")
            .into()
    }

    #[test]
    fn legacy_inherited_status_path_is_disabled() {
        assert_eq!(run(&[]), EXIT_RUNTIME_SERVICE_REQUIRED);
        assert_eq!(
            run(&[
                "--rows".to_owned(),
                "24".to_owned(),
                "--cols".to_owned(),
                "80".to_owned(),
            ]),
            EXIT_RUNTIME_SERVICE_REQUIRED
        );
    }

    #[test]
    fn policy_is_bounded_and_never_resolves_a_host_shell() {
        let owner = owner();
        let canary = "private-argument-canary";
        let spec = TtyOneShotSpec::new(
            &owner,
            24,
            80,
            vec!["/bin/tool".to_owned(), canary.to_owned()],
            vec!["PATH=/bin".to_owned(), "PRIVATE=secret".to_owned()],
        )
        .unwrap();
        assert_eq!(spec.rows(), 24);
        assert_eq!(spec.cols(), 80);
        assert!(!format!("{spec:?}").contains(canary));
        assert!(!format!("{spec:?}").contains("secret"));
        assert_eq!(
            TtyOneShotSpec::new(
                &owner,
                24,
                80,
                vec!["sh".to_owned()],
                vec!["PATH=/bin".to_owned()]
            ),
            Err(TtyOneShotError::InvalidPolicy)
        );
        assert_eq!(
            TtyOneShotSpec::new(
                &owner,
                24,
                80,
                std::iter::once("/bin/tool".to_owned())
                    .chain((0..MAX_ARGV_ENTRIES).map(|_| "argument".to_owned()))
                    .collect(),
                vec!["PATH=/bin".to_owned()]
            ),
            Err(TtyOneShotError::InvalidPolicy)
        );
        assert_eq!(
            TtyOneShotSpec::new(
                &owner,
                24,
                80,
                vec!["/bin/tool".to_owned()],
                vec![format!(
                    "PRIVATE={}",
                    "x".repeat(crate::environment::MAX_MANAGER_ENVIRONMENT_BYTES)
                )]
            ),
            Err(TtyOneShotError::InvalidPolicy)
        );
        assert_eq!(
            TtyOneShotSpec::new(
                &owner,
                24,
                80,
                vec!["/bin/sh".to_owned()],
                vec!["BAD".to_owned()]
            ),
            Err(TtyOneShotError::InvalidPolicy)
        );
    }

    #[test]
    fn descriptor_requires_authenticated_owner_and_real_cloexec_terminal() {
        let owner = owner();
        let request = request(1);
        let mut attachment = terminal_attachment(&owner, &request);
        attachment.owner_uid = owner.uid().saturating_add(1);
        assert_eq!(
            ValidatedTerminal::new(&owner, &request, &attachment, terminal_fd()).map(|_| ()),
            Err(TtyOneShotError::AttachmentMismatch)
        );

        let mut attachment = terminal_attachment(&owner, &request);
        attachment.cloexec = false;
        assert_eq!(
            ValidatedTerminal::new(&owner, &request, &attachment, terminal_fd()).map(|_| ()),
            Err(TtyOneShotError::AttachmentMismatch)
        );

        let attachment = terminal_attachment(&owner, &request);
        let ordinary_file: OwnedFd = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap()
            .into();
        assert_eq!(
            ValidatedTerminal::new(&owner, &request, &attachment, ordinary_file).map(|_| ()),
            Err(TtyOneShotError::AttachmentMismatch)
        );

        let terminal = terminal_fd();
        rustix::io::fcntl_setfd(&terminal, FdFlags::empty()).unwrap();
        assert_eq!(
            ValidatedTerminal::new(&owner, &request, &attachment, terminal).map(|_| ()),
            Err(TtyOneShotError::AttachmentMismatch)
        );
    }

    #[test]
    fn start_and_cancel_use_exact_transient_user_scope() {
        let owner = owner();
        let request = request(2);
        let attachment = terminal_attachment(&owner, &request);
        let mut service = TtyOneShotService::new(owner.clone(), Backend::default());
        service
            .start(&request, &spec(&owner), &attachment, terminal_fd())
            .unwrap();
        assert_eq!(
            service.cancel(owner.session_generation(), request.request_id()),
            Ok(CancelOutcome::TeardownComplete)
        );
        assert_eq!(
            service.cancel(owner.session_generation(), request.request_id()),
            Ok(CancelOutcome::AlreadyTerminal)
        );
        service.teardown_all().unwrap();
        assert_eq!(service.backend().starts, 1);
        assert_eq!(service.backend().teardowns, 1);
    }

    #[test]
    fn scope_mismatch_is_torn_down_and_never_adopted() {
        let owner = owner();
        let request = request(3);
        let attachment = terminal_attachment(&owner, &request);
        let mut service = TtyOneShotService::new(
            owner.clone(),
            Backend {
                wrong_owner: true,
                ..Backend::default()
            },
        );
        assert_eq!(
            service
                .start(&request, &spec(&owner), &attachment, terminal_fd())
                .map(|_| ()),
            Err(TtyOneShotError::ScopeOwnershipMismatch)
        );
        service.teardown_all().unwrap();
        assert_eq!(service.backend().starts, 1);
        assert_eq!(service.backend().teardowns, 1);
    }

    #[test]
    fn failed_teardown_remains_active_and_fails_closed() {
        let owner = owner();
        let request = request(4);
        let attachment = terminal_attachment(&owner, &request);
        let mut service = TtyOneShotService::new(
            owner.clone(),
            Backend {
                fail_teardown: true,
                ..Backend::default()
            },
        );
        service
            .start(&request, &spec(&owner), &attachment, terminal_fd())
            .unwrap();
        assert_eq!(
            service.cancel(owner.session_generation(), request.request_id()),
            Err(TtyOneShotError::TeardownFailed)
        );
        assert_eq!(
            service.cancel(owner.session_generation(), request.request_id()),
            Err(TtyOneShotError::TeardownFailed)
        );
        assert_eq!(service.active.len(), 1);
    }
}
