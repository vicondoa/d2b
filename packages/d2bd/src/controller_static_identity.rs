use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::Arc,
};

use d2b_host::guest_runtime::{
    CONTROLLER_SESSION_GENERATION_ENV, CONTROLLER_STATIC_IDENTITY_CREDENTIAL,
    CONTROLLER_STATIC_IDENTITY_FD_ENV,
};
use d2b_host::realm_controller_bootstrap::{
    OVERFLOW_ID, REALM_CONTROLLER_AUTHORITY_FD_ENV, REALM_CONTROLLER_AUTHORITY_RESOURCE_ID,
    RealmControllerChildAuthority,
};
use d2b_session_unix::{SeqpacketSocket, UnixSessionError};
use nix::fcntl::{FcntlArg, FdFlag, OFlag, SealFlag, fcntl};
use zeroize::{Zeroize, Zeroizing};

const STATIC_KEY_BYTES: usize = 32;
const CHILD_ROLE_ENV: &str = "D2B_CHILD_ROLE";
const REALM_ID_ENV: &str = "D2B_REALM_ID";
const CONTROLLER_GENERATION_ENV: &str = "D2B_CONTROLLER_GENERATION";
const PROCESS_ID_ENV: &str = "D2B_PROCESS_ID";
const CGROUP_DIGEST_ENV: &str = "D2B_CGROUP_DIGEST";
const MAX_AUTHORITY_BYTES: u64 = 1024;
const MAX_ID_MAP_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerIdentityError {
    Missing,
    InvalidBootstrap,
    UnsafeDescriptor,
    InvalidKey,
}

impl fmt::Display for ControllerIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "controller-static-identity-unavailable",
            Self::InvalidBootstrap => "controller-runtime-bootstrap-invalid",
            Self::UnsafeDescriptor => "controller-static-identity-descriptor-unsafe",
            Self::InvalidKey => "controller-static-identity-invalid",
        })
    }
}

impl std::error::Error for ControllerIdentityError {}

#[derive(Clone, PartialEq, Eq)]
pub struct ControllerProcessBinding {
    realm_id: String,
    generation: u64,
    controller_uid: u32,
    controller_gid: u32,
    broker_host_uid: Option<u32>,
    broker_host_gid: Option<u32>,
    broker_namespace_uid: Option<u32>,
    broker_namespace_gid: Option<u32>,
    child_realm: bool,
}

impl ControllerProcessBinding {
    pub fn from_process(
        local_root_generation: u64,
        controller_uid: u32,
        controller_gid: u32,
    ) -> Result<Self, ControllerIdentityError> {
        if local_root_generation == 0 {
            return Err(ControllerIdentityError::InvalidBootstrap);
        }
        match std::env::var(CHILD_ROLE_ENV) {
            Ok(role) => {
                if role != "controller"
                    || controller_uid != 0
                    || controller_gid != 0
                    || rustix::process::geteuid().as_raw() != 0
                    || rustix::process::getegid().as_raw() != 0
                {
                    return Err(ControllerIdentityError::InvalidBootstrap);
                }
                let authority = load_controller_authority()?;
                validate_controller_environment(&authority)?;
                let uid_map = load_id_map("/proc/self/uid_map")?;
                let gid_map = load_id_map("/proc/self/gid_map")?;
                uid_map.verify(0, authority.controller_host_uid)?;
                gid_map.verify(0, authority.controller_host_gid)?;
                uid_map.verify(authority.broker_namespace_uid, authority.broker_host_uid)?;
                gid_map.verify(authority.broker_namespace_gid, authority.broker_host_gid)?;
                if authority.broker_namespace_uid == OVERFLOW_ID
                    || authority.broker_namespace_gid == OVERFLOW_ID
                {
                    return Err(ControllerIdentityError::InvalidBootstrap);
                }
                Ok(Self {
                    realm_id: authority.realm_id,
                    generation: authority.session_generation,
                    controller_uid: authority.controller_host_uid,
                    controller_gid: authority.controller_host_gid,
                    broker_host_uid: Some(authority.broker_host_uid),
                    broker_host_gid: Some(authority.broker_host_gid),
                    broker_namespace_uid: Some(authority.broker_namespace_uid),
                    broker_namespace_gid: Some(authority.broker_namespace_gid),
                    child_realm: true,
                })
            }
            Err(std::env::VarError::NotPresent) => {
                if rustix::process::geteuid().as_raw() != controller_uid
                    || rustix::process::getegid().as_raw() != controller_gid
                {
                    return Err(ControllerIdentityError::InvalidBootstrap);
                }
                Ok(Self {
                    realm_id: "local-root".to_owned(),
                    generation: local_root_generation,
                    controller_uid,
                    controller_gid,
                    broker_host_uid: None,
                    broker_host_gid: None,
                    broker_namespace_uid: None,
                    broker_namespace_gid: None,
                    child_realm: false,
                })
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(ControllerIdentityError::InvalidBootstrap)
            }
        }
    }

    pub fn realm_id(&self) -> &str {
        &self.realm_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn controller_uid(&self) -> u32 {
        self.controller_uid
    }

    pub const fn controller_gid(&self) -> u32 {
        self.controller_gid
    }

    pub const fn is_child_realm(&self) -> bool {
        self.child_realm
    }

    fn expected_broker_peer_ids(
        &self,
        broker_uid: u32,
        broker_gid: u32,
    ) -> Result<(u32, u32), ControllerIdentityError> {
        if self.child_realm {
            if self.broker_host_uid != Some(broker_uid) || self.broker_host_gid != Some(broker_gid)
            {
                return Err(ControllerIdentityError::InvalidBootstrap);
            }
            return Ok((
                self.broker_namespace_uid
                    .ok_or(ControllerIdentityError::InvalidBootstrap)?,
                self.broker_namespace_gid
                    .ok_or(ControllerIdentityError::InvalidBootstrap)?,
            ));
        }
        Ok((broker_uid, broker_gid))
    }

    pub(crate) fn verify_broker_peer(
        &self,
        peer: &SeqpacketSocket,
        broker_uid: u32,
        broker_gid: u32,
    ) -> Result<(), UnixSessionError> {
        let (expected_uid, expected_gid) = self
            .expected_broker_peer_ids(broker_uid, broker_gid)
            .map_err(|_| UnixSessionError::CredentialMismatch)?;
        let credentials = peer.acceptor_peer_credentials()?;
        if credentials.uid().as_raw() == expected_uid && credentials.gid().as_raw() == expected_gid
        {
            Ok(())
        } else {
            Err(UnixSessionError::CredentialMismatch)
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        realm_id: impl Into<String>,
        generation: u64,
        controller_uid: u32,
        controller_gid: u32,
    ) -> Self {
        Self {
            realm_id: realm_id.into(),
            generation,
            controller_uid,
            controller_gid,
            broker_host_uid: Some(controller_uid),
            broker_host_gid: Some(controller_gid),
            broker_namespace_uid: Some(controller_uid),
            broker_namespace_gid: Some(controller_gid),
            child_realm: true,
        }
    }
}

impl fmt::Debug for ControllerProcessBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControllerProcessBinding")
            .field("realm", &"<redacted>")
            .field("generation", &"<redacted>")
            .field("child_realm", &self.child_realm)
            .finish()
    }
}

pub struct ControllerStaticIdentity {
    binding: ControllerProcessBinding,
    private_key: Zeroizing<[u8; STATIC_KEY_BYTES]>,
    public_key: [u8; STATIC_KEY_BYTES],
}

impl ControllerStaticIdentity {
    fn from_key(
        binding: ControllerProcessBinding,
        mut private_key: [u8; STATIC_KEY_BYTES],
    ) -> Result<Self, ControllerIdentityError> {
        let owned_private_key = Zeroizing::new(private_key);
        private_key.zeroize();
        let public_key = d2b_session::x25519_public_key(&owned_private_key)
            .map_err(|_| ControllerIdentityError::InvalidKey)?;
        if public_key == [0; STATIC_KEY_BYTES] {
            return Err(ControllerIdentityError::InvalidKey);
        }
        Ok(Self {
            binding,
            private_key: owned_private_key,
            public_key,
        })
    }

    pub fn binding(&self) -> &ControllerProcessBinding {
        &self.binding
    }

    pub const fn public_key(&self) -> &[u8; STATIC_KEY_BYTES] {
        &self.public_key
    }

    pub fn handshake_secret(&self) -> Result<d2b_session::Secret32, ControllerIdentityError> {
        let mut copy = *self.private_key;
        let secret =
            d2b_session::Secret32::new(copy).map_err(|_| ControllerIdentityError::InvalidKey);
        zeroize::Zeroize::zeroize(&mut copy);
        secret
    }
}

impl fmt::Debug for ControllerStaticIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ControllerStaticIdentity(REDACTED)")
    }
}

#[derive(Clone)]
pub struct ControllerIdentityAuthority {
    binding: ControllerProcessBinding,
    identity: Option<Arc<ControllerStaticIdentity>>,
    unavailable: Option<ControllerIdentityError>,
}

impl ControllerIdentityAuthority {
    pub fn load(binding: ControllerProcessBinding) -> Self {
        let loaded = if binding.is_child_realm() {
            load_inherited_identity(&binding)
        } else {
            load_systemd_identity(&binding)
        };
        match loaded {
            Ok(identity) => Self {
                binding,
                identity: Some(Arc::new(identity)),
                unavailable: None,
            },
            Err(error) => Self {
                binding,
                identity: None,
                unavailable: Some(error),
            },
        }
    }

    pub fn binding(&self) -> &ControllerProcessBinding {
        &self.binding
    }

    pub fn require(&self) -> Result<Arc<ControllerStaticIdentity>, ControllerIdentityError> {
        self.identity
            .as_ref()
            .map(Arc::clone)
            .ok_or(self.unavailable.unwrap_or(ControllerIdentityError::Missing))
    }

    pub const fn available(&self) -> bool {
        self.identity.is_some()
    }

    #[cfg(test)]
    pub(crate) fn from_test_key(binding: ControllerProcessBinding, private_key: [u8; 32]) -> Self {
        let identity = ControllerStaticIdentity::from_key(binding.clone(), private_key)
            .expect("valid test controller identity");
        Self {
            binding,
            identity: Some(Arc::new(identity)),
            unavailable: None,
        }
    }
}

impl fmt::Debug for ControllerIdentityAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControllerIdentityAuthority")
            .field("binding", &self.binding)
            .field("available", &self.available())
            .finish()
    }
}

fn load_controller_authority() -> Result<RealmControllerChildAuthority, ControllerIdentityError> {
    let raw_fd = std::env::var(REALM_CONTROLLER_AUTHORITY_FD_ENV)
        .ok()
        .and_then(|value| value.parse::<RawFd>().ok())
        .filter(|fd| (10..=4096).contains(fd))
        .ok_or(ControllerIdentityError::Missing)?;
    verify_resource_binding(raw_fd, REALM_CONTROLLER_AUTHORITY_RESOURCE_ID)?;
    let path = PathBuf::from(format!("/proc/self/fd/{raw_fd}"));
    let reopened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path);
    let _ = nix::unistd::close(raw_fd);
    let file = reopened.map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    let status = OFlag::from_bits_truncate(
        fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
            .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
    );
    let descriptor = FdFlag::from_bits_truncate(
        fcntl(file.as_raw_fd(), FcntlArg::F_GETFD)
            .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
    );
    let seals = SealFlag::from_bits_truncate(
        fcntl(file.as_raw_fd(), FcntlArg::F_GET_SEALS)
            .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
    );
    let required = SealFlag::F_SEAL_WRITE
        | SealFlag::F_SEAL_GROW
        | SealFlag::F_SEAL_SHRINK
        | SealFlag::F_SEAL_SEAL;
    if status & OFlag::O_ACCMODE != OFlag::O_RDONLY
        || !descriptor.contains(FdFlag::FD_CLOEXEC)
        || !seals.contains(required)
    {
        return Err(ControllerIdentityError::UnsafeDescriptor);
    }
    let mut encoded = Vec::new();
    file.take(MAX_AUTHORITY_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    if encoded.is_empty() || encoded.len() as u64 > MAX_AUTHORITY_BYTES {
        return Err(ControllerIdentityError::UnsafeDescriptor);
    }
    RealmControllerChildAuthority::decode(&encoded)
        .map_err(|_| ControllerIdentityError::InvalidBootstrap)
}

fn verify_resource_binding(fd: RawFd, expected_id: &str) -> Result<(), ControllerIdentityError> {
    let expected_fd = fd.to_string();
    let matched = (0..128).any(|index| {
        let fd_key = format!("D2B_RESOURCE_FD_{index}");
        let id_key = format!("{fd_key}_ID");
        std::env::var(&fd_key).ok().as_deref() == Some(expected_fd.as_str())
            && std::env::var(&id_key).ok().as_deref() == Some(expected_id)
    });
    if matched {
        Ok(())
    } else {
        Err(ControllerIdentityError::InvalidBootstrap)
    }
}

fn validate_controller_environment(
    authority: &RealmControllerChildAuthority,
) -> Result<(), ControllerIdentityError> {
    let realm = std::env::var(REALM_ID_ENV)
        .ok()
        .filter(|value| valid_realm_id(value))
        .ok_or(ControllerIdentityError::InvalidBootstrap)?;
    let generation = std::env::var(CONTROLLER_SESSION_GENERATION_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value != 0)
        .ok_or(ControllerIdentityError::InvalidBootstrap)?;
    let cgroup_digest = std::env::var(CGROUP_DIGEST_ENV)
        .ok()
        .and_then(|value| decode_digest(&value))
        .ok_or(ControllerIdentityError::InvalidBootstrap)?;
    if authority.realm_id != realm
        || authority.controller_generation
            != std::env::var(CONTROLLER_GENERATION_ENV)
                .map_err(|_| ControllerIdentityError::InvalidBootstrap)?
        || authority.controller_process_id
            != std::env::var(PROCESS_ID_ENV)
                .map_err(|_| ControllerIdentityError::InvalidBootstrap)?
        || authority.session_generation != generation
        || authority.cgroup_digest != cgroup_digest
    {
        return Err(ControllerIdentityError::InvalidBootstrap);
    }
    Ok(())
}

fn decode_digest(encoded: &str) -> Option<[u8; 32]> {
    if encoded.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

#[derive(Clone, Copy)]
struct IdMapEntry {
    namespace_start: u32,
    host_start: u32,
    length: u32,
}

struct IdMap {
    entries: Vec<IdMapEntry>,
}

impl IdMap {
    fn parse(encoded: &str) -> Result<Self, ControllerIdentityError> {
        let mut entries = Vec::new();
        for line in encoded.lines() {
            let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(ControllerIdentityError::InvalidBootstrap);
            }
            let parse = |field: &str| {
                field
                    .parse::<u32>()
                    .map_err(|_| ControllerIdentityError::InvalidBootstrap)
            };
            let entry = IdMapEntry {
                namespace_start: parse(fields[0])?,
                host_start: parse(fields[1])?,
                length: parse(fields[2])?,
            };
            if entry.length == 0
                || entry.namespace_start.checked_add(entry.length).is_none()
                || entry.host_start.checked_add(entry.length).is_none()
                || entries.iter().any(|prior: &IdMapEntry| {
                    ranges_overlap(
                        prior.namespace_start,
                        prior.length,
                        entry.namespace_start,
                        entry.length,
                    ) || ranges_overlap(
                        prior.host_start,
                        prior.length,
                        entry.host_start,
                        entry.length,
                    )
                })
            {
                return Err(ControllerIdentityError::InvalidBootstrap);
            }
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(ControllerIdentityError::InvalidBootstrap);
        }
        Ok(Self { entries })
    }

    fn verify(&self, namespace_id: u32, host_id: u32) -> Result<(), ControllerIdentityError> {
        if namespace_id == OVERFLOW_ID
            || !self.entries.iter().any(|entry| {
                entry.namespace_start == namespace_id
                    && entry.host_start == host_id
                    && entry.length == 1
            })
        {
            return Err(ControllerIdentityError::InvalidBootstrap);
        }
        Ok(())
    }
}

fn ranges_overlap(left_start: u32, left_len: u32, right_start: u32, right_len: u32) -> bool {
    left_start < right_start + right_len && right_start < left_start + left_len
}

fn load_id_map(path: &'static str) -> Result<IdMap, ControllerIdentityError> {
    let mut encoded = String::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_ID_MAP_BYTES + 1)
                .read_to_string(&mut encoded)
                .map(|_| ())
        })
        .map_err(|_| ControllerIdentityError::InvalidBootstrap)?;
    if encoded.is_empty() || encoded.len() as u64 > MAX_ID_MAP_BYTES {
        return Err(ControllerIdentityError::InvalidBootstrap);
    }
    IdMap::parse(&encoded)
}

fn load_inherited_identity(
    binding: &ControllerProcessBinding,
) -> Result<ControllerStaticIdentity, ControllerIdentityError> {
    let raw_fd = std::env::var(CONTROLLER_STATIC_IDENTITY_FD_ENV)
        .ok()
        .and_then(|value| value.parse::<RawFd>().ok())
        .filter(|fd| (10..=4096).contains(fd))
        .ok_or(ControllerIdentityError::Missing)?;
    let path = PathBuf::from(format!("/proc/self/fd/{raw_fd}"));
    let reopened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path);
    let _ = nix::unistd::close(raw_fd);
    let file = reopened.map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    load_file(binding.clone(), file, true)
}

fn load_systemd_identity(
    binding: &ControllerProcessBinding,
) -> Result<ControllerStaticIdentity, ControllerIdentityError> {
    let directory = std::env::var_os("CREDENTIALS_DIRECTORY")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(ControllerIdentityError::Missing)?;
    let directory_metadata =
        std::fs::symlink_metadata(&directory).map_err(|_| ControllerIdentityError::Missing)?;
    if directory_metadata.file_type().is_symlink()
        || !directory_metadata.is_dir()
        || directory_metadata.mode() & 0o022 != 0
    {
        return Err(ControllerIdentityError::UnsafeDescriptor);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(directory.join(CONTROLLER_STATIC_IDENTITY_CREDENTIAL))
        .map_err(|_| ControllerIdentityError::Missing)?;
    load_file(binding.clone(), file, false)
}

fn load_file(
    binding: ControllerProcessBinding,
    mut file: File,
    require_seals: bool,
) -> Result<ControllerStaticIdentity, ControllerIdentityError> {
    let status = OFlag::from_bits_truncate(
        fcntl(file.as_raw_fd(), FcntlArg::F_GETFL)
            .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
    );
    let descriptor = FdFlag::from_bits_truncate(
        fcntl(file.as_raw_fd(), FcntlArg::F_GETFD)
            .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
    );
    let metadata = file
        .metadata()
        .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    if status & OFlag::O_ACCMODE != OFlag::O_RDONLY
        || !descriptor.contains(FdFlag::FD_CLOEXEC)
        || !metadata.is_file()
        || metadata.len() != STATIC_KEY_BYTES as u64
    {
        return Err(ControllerIdentityError::UnsafeDescriptor);
    }
    if require_seals {
        let seals = SealFlag::from_bits_truncate(
            fcntl(file.as_raw_fd(), FcntlArg::F_GET_SEALS)
                .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?,
        );
        let required = SealFlag::F_SEAL_WRITE
            | SealFlag::F_SEAL_GROW
            | SealFlag::F_SEAL_SHRINK
            | SealFlag::F_SEAL_SEAL;
        if !seals.contains(required) {
            return Err(ControllerIdentityError::UnsafeDescriptor);
        }
    } else {
        let owner = metadata.uid();
        if (owner != 0 && owner != binding.controller_uid()) || metadata.mode() & 0o277 != 0 {
            return Err(ControllerIdentityError::UnsafeDescriptor);
        }
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(STATIC_KEY_BYTES + 1));
    file.take((STATIC_KEY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ControllerIdentityError::UnsafeDescriptor)?;
    let private_key: [u8; STATIC_KEY_BYTES] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ControllerIdentityError::InvalidKey)?;
    ControllerStaticIdentity::from_key(binding, private_key)
}

fn valid_realm_id(value: &str) -> bool {
    let mut chars = value.chars();
    !value.is_empty()
        && value.len() <= 64
        && chars.next().is_some_and(|ch| ch.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        os::fd::{AsRawFd, OwnedFd},
        process::Command,
        time::{Duration, Instant},
    };

    use super::*;
    use nix::sys::socket::{
        AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept4, bind, connect,
        listen, recv, send, socket, socketpair,
    };

    fn binding() -> ControllerProcessBinding {
        ControllerProcessBinding {
            realm_id: "work".to_owned(),
            generation: 7,
            controller_uid: 1000,
            controller_gid: 1000,
            broker_host_uid: Some(1001),
            broker_host_gid: Some(1001),
            broker_namespace_uid: Some(1),
            broker_namespace_gid: Some(1),
            child_realm: true,
        }
    }

    fn sealed_key(bytes: &[u8]) -> File {
        let fd = rustix::fs::memfd_create(
            "controller-static-test",
            rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
        )
        .unwrap();
        let mut writer = File::from(fd);
        writer.write_all(bytes).unwrap();
        writer.seek(SeekFrom::Start(0)).unwrap();
        fcntl(
            writer.as_raw_fd(),
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_WRITE
                    | SealFlag::F_SEAL_GROW
                    | SealFlag::F_SEAL_SHRINK
                    | SealFlag::F_SEAL_SEAL,
            ),
        )
        .unwrap();
        let readonly = rustix::fs::open(
            format!("/proc/self/fd/{}", writer.as_raw_fd()),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        File::from(readonly)
    }

    fn sealed_controller_authority(authority: &RealmControllerChildAuthority) -> OwnedFd {
        let encoded = authority.encode().unwrap();
        let fd = rustix::fs::memfd_create(
            "controller-authority-test",
            rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
        )
        .unwrap();
        let mut writer = File::from(fd);
        writer.write_all(&encoded).unwrap();
        writer.seek(SeekFrom::Start(0)).unwrap();
        fcntl(
            writer.as_raw_fd(),
            FcntlArg::F_ADD_SEALS(
                SealFlag::F_SEAL_WRITE
                    | SealFlag::F_SEAL_GROW
                    | SealFlag::F_SEAL_SHRINK
                    | SealFlag::F_SEAL_SEAL,
            ),
        )
        .unwrap();
        let readonly = rustix::fs::open(
            format!("/proc/self/fd/{}", writer.as_raw_fd()),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        drop(writer);
        readonly
    }

    fn clear_cloexec(fd: &OwnedFd) {
        let flags = FdFlag::from_bits_truncate(fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).unwrap());
        fcntl(
            fd.as_raw_fd(),
            FcntlArg::F_SETFD(flags - FdFlag::FD_CLOEXEC),
        )
        .unwrap();
    }

    fn unprivileged_user_namespace_available() -> bool {
        match Command::new("unshare")
            .args(["--user", "--map-root-user", "--", "true"])
            .output()
        {
            Ok(output) if output.status.success() => true,
            Ok(output)
                if String::from_utf8_lossy(&output.stderr).contains("Operation not permitted")
                    && String::from_utf8_lossy(&output.stderr).contains("uid_map") =>
            {
                eprintln!(
                    "skipping controller identity process test: user namespace unavailable: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                false
            }
            Ok(output) => panic!(
                "unprivileged user namespace probe failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipping controller identity process test: unshare unavailable");
                false
            }
            Err(error) => panic!("failed to probe unprivileged user namespaces: {error}"),
        }
    }

    #[test]
    fn sealed_exact_key_loads_and_debug_is_redacted() {
        let identity = load_file(binding(), sealed_key(&[7; 32]), true).unwrap();
        assert_ne!(identity.public_key(), &[0; 32]);
        let debug = format!("{identity:?}");
        assert_eq!(debug, "ControllerStaticIdentity(REDACTED)");
        assert!(!debug.contains('7'));
    }

    #[test]
    fn missing_seals_wrong_length_and_zero_key_fail_closed() {
        let unsealed = rustix::fs::memfd_create(
            "controller-static-unsealed",
            rustix::fs::MemfdFlags::CLOEXEC,
        )
        .map(File::from)
        .unwrap();
        assert_eq!(
            load_file(binding(), unsealed, true).unwrap_err(),
            ControllerIdentityError::UnsafeDescriptor
        );
        assert_eq!(
            load_file(binding(), sealed_key(&[1; 31]), true).unwrap_err(),
            ControllerIdentityError::UnsafeDescriptor
        );
        assert_eq!(
            load_file(binding(), sealed_key(&[0; 32]), true).unwrap_err(),
            ControllerIdentityError::InvalidKey
        );
    }

    #[test]
    fn controller_rejects_overflow_or_unmapped_broker_ids() {
        let mapped = IdMap::parse("0 1000 1\n1 1001 1\n").unwrap();
        mapped.verify(0, 1000).unwrap();
        mapped.verify(1, 1001).unwrap();
        let map = IdMap::parse("0 1000 1\n65534 1001 1\n").unwrap();
        assert_eq!(
            map.verify(OVERFLOW_ID, 1001),
            Err(ControllerIdentityError::InvalidBootstrap)
        );
        assert_eq!(
            map.verify(1, 1001),
            Err(ControllerIdentityError::InvalidBootstrap)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_root_uses_host_peer_ids_and_cross_mode_is_rejected() {
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let local = ControllerProcessBinding {
            realm_id: "local-root".to_owned(),
            generation: 7,
            controller_uid: uid,
            controller_gid: gid,
            broker_host_uid: None,
            broker_host_gid: None,
            broker_namespace_uid: None,
            broker_namespace_gid: None,
            child_realm: false,
        };
        assert_eq!(
            local.expected_broker_peer_ids(uid, gid).unwrap(),
            (uid, gid)
        );
        let (left, _right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        )
        .unwrap();
        let peer = SeqpacketSocket::from_owned(left).unwrap();
        local.verify_broker_peer(&peer, uid, gid).unwrap();

        let child = binding();
        assert_eq!(child.expected_broker_peer_ids(1001, 1001).unwrap(), (1, 1));
        assert_eq!(
            child.expected_broker_peer_ids(1002, 1002),
            Err(ControllerIdentityError::InvalidBootstrap)
        );
        if uid != 1 || gid != 1 {
            assert_eq!(
                child.verify_broker_peer(&peer, 1001, 1001),
                Err(UnixSessionError::CredentialMismatch)
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_broker_peer_helper() {
        if std::env::var_os("D2B_CONTROLLER_BROKER_HELPER").is_none() {
            return;
        }
        let binding = ControllerProcessBinding::from_process(1, 0, 0).unwrap();
        let path = std::env::var_os("D2B_CONTROLLER_BROKER_SOCKET").unwrap();
        let socket_fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .unwrap();
        connect(
            socket_fd.as_raw_fd(),
            &UnixAddr::new(PathBuf::from(path).as_path()).unwrap(),
        )
        .unwrap();
        let ready_fd = socket_fd.try_clone().unwrap();
        let peer = SeqpacketSocket::from_owned(socket_fd).unwrap();
        binding
            .verify_broker_peer(
                &peer,
                binding.broker_host_uid.unwrap(),
                binding.broker_host_gid.unwrap(),
            )
            .unwrap();
        send(ready_fd.as_raw_fd(), b"ready", MsgFlags::MSG_NOSIGNAL).unwrap();
    }

    #[test]
    fn spawned_controller_authenticates_mapped_broker_peer() {
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        if uid == 0 || gid == 0 {
            return;
        }
        if !unprivileged_user_namespace_available() {
            return;
        }
        let root = tempfile::tempdir_in(crate::test_socket_root("controller-identity"))
            .expect("create controller identity socket tempdir");
        let socket_path = root.path().join("broker.sock");
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .unwrap();
        bind(listener.as_raw_fd(), &UnixAddr::new(&socket_path).unwrap()).unwrap();
        listen(&listener, Backlog::new(1).unwrap()).unwrap();
        let authority = RealmControllerChildAuthority {
            realm_id: "work".to_owned(),
            controller_generation: "generation-1".to_owned(),
            controller_process_id: "controller-1".to_owned(),
            session_generation: 7,
            controller_host_uid: uid,
            controller_host_gid: gid,
            broker_host_uid: uid,
            broker_host_gid: gid,
            broker_namespace_uid: 0,
            broker_namespace_gid: 0,
            cgroup_digest: [9; 32],
        };
        let authority_fd =
            rustix::io::fcntl_dupfd_cloexec(sealed_controller_authority(&authority), 10).unwrap();
        clear_cloexec(&authority_fd);
        let mut child = Command::new("unshare")
            .args(["--user", "--map-root-user", "--"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "controller_static_identity::tests::controller_broker_peer_helper",
                "--nocapture",
            ])
            .env_clear()
            .env("PATH", "/run/current-system/sw/bin:/usr/bin:/bin")
            .env("D2B_CONTROLLER_BROKER_HELPER", "1")
            .env("D2B_CONTROLLER_BROKER_SOCKET", &socket_path)
            .env("D2B_CHILD_ROLE", "controller")
            .env("D2B_REALM_ID", "work")
            .env("D2B_CONTROLLER_GENERATION", "generation-1")
            .env("D2B_CONTROLLER_SESSION_GENERATION", "7")
            .env("D2B_PROCESS_ID", "controller-1")
            .env(
                "D2B_CGROUP_DIGEST",
                "0909090909090909090909090909090909090909090909090909090909090909",
            )
            .env(
                "D2B_REALM_CONTROLLER_AUTHORITY_FD",
                authority_fd.as_raw_fd().to_string(),
            )
            .env("D2B_RESOURCE_FD_0", authority_fd.as_raw_fd().to_string())
            .env("D2B_RESOURCE_FD_0_ID", "realm-controller-authority-v1")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let accepted = loop {
            match accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC) {
                Ok(fd) => break fd,
                Err(nix::errno::Errno::EAGAIN) if Instant::now() < deadline => {
                    assert!(child.try_wait().unwrap().is_none());
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("controller did not connect: {error}"),
            }
        };
        let mut ready = [0_u8; 8];
        let count = recv(accepted, &mut ready, MsgFlags::empty()).unwrap();
        assert_eq!(&ready[..count], b"ready");
        nix::unistd::close(accepted).unwrap();
        assert!(child.wait().unwrap().success());
    }
}
