//! Parent-spawned child-realm broker process runtime.

use std::{
    env, fmt,
    fs::File,
    io::Read,
    os::fd::{AsFd, AsRawFd, OwnedFd, RawFd},
    sync::Arc,
};

use d2b_host::{
    realm_broker_bootstrap::{
        REALM_BROKER_AUTHORITY_FD_ENV, REALM_BROKER_AUTHORITY_RESOURCE_ID,
        REALM_BROKER_GUEST_RUNTIME_FD_ENV, REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID,
        RealmBrokerChildAuthority, RealmBrokerGuestRuntimeBootstrap,
    },
    realm_children::RealmChildFdKind,
};
use nix::{
    fcntl::{FcntlArg, FdFlag, OFlag, SealFlag, fcntl},
    sys::socket::{MsgFlags, SockFlag, accept4, send},
};

use crate::{
    child_realm_guest_material::build_guest_material_handler, runtime::RunError,
    service_v2::RealmBrokerSessionBinding,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const MAX_AUTHORITY_BYTES: u64 = 1024;
const MAX_GUEST_RUNTIME_BYTES: u64 = 4 * 1024 * 1024;
const LISTENER_FD_ENV: &str = "D2B_BROKER_LISTENER_FD";
const BOOTSTRAP_FD_ENV: &str = "D2B_BOOTSTRAP_SESSION_FD";
const CGROUP_FD_ENV: &str = "D2B_CGROUP_LEAF_FD";
const REALM_ENV: &str = "D2B_REALM_ID";
const GENERATION_ENV: &str = "D2B_CONTROLLER_GENERATION";
const SESSION_GENERATION_ENV: &str = "D2B_CONTROLLER_SESSION_GENERATION";
const PROCESS_ID_ENV: &str = "D2B_PROCESS_ID";
const ROLE_ENV: &str = "D2B_CHILD_ROLE";
const CGROUP_DIGEST_ENV: &str = "D2B_CGROUP_DIGEST";
const MAX_ID_MAP_BYTES: u64 = 4096;

pub struct ChildRealmBrokerConfig {
    listener: OwnedFd,
    bootstrap: OwnedFd,
    _cgroup: OwnedFd,
    authority: RealmBrokerChildAuthority,
    guest_runtime: RealmBrokerGuestRuntimeBootstrap,
    controller_namespace_uid: u32,
    controller_namespace_gid: u32,
}

impl fmt::Debug for ChildRealmBrokerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildRealmBrokerConfig(REDACTED)")
    }
}

impl ChildRealmBrokerConfig {
    pub fn from_environment() -> Result<Self, RunError> {
        if env::var_os("LISTEN_FDS").is_some()
            || env::var_os("LISTEN_PID").is_some()
            || env::var_os("LISTEN_FDNAMES").is_some()
        {
            return Err(protocol("child realm broker rejects systemd activation"));
        }
        if env_value(ROLE_ENV)? != "broker" {
            return Err(protocol("child realm broker role mismatch"));
        }
        let listener_raw = env_fd(LISTENER_FD_ENV)?;
        let bootstrap_raw = env_fd(BOOTSTRAP_FD_ENV)?;
        let cgroup_raw = env_fd(CGROUP_FD_ENV)?;
        let authority_raw = env_fd(REALM_BROKER_AUTHORITY_FD_ENV)?;
        let guest_runtime_raw = env_fd(REALM_BROKER_GUEST_RUNTIME_FD_ENV)?;
        verify_resource_binding(authority_raw, REALM_BROKER_AUTHORITY_RESOURCE_ID)?;
        verify_resource_binding(guest_runtime_raw, REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID)?;
        let mut distinct = [
            listener_raw,
            bootstrap_raw,
            cgroup_raw,
            authority_raw,
            guest_runtime_raw,
        ];
        distinct.sort_unstable();
        if distinct.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(protocol("child realm broker descriptor alias"));
        }

        let listener = adopt_fd(listener_raw)?;
        let bootstrap = adopt_fd(bootstrap_raw)?;
        let cgroup = adopt_fd(cgroup_raw)?;
        let authority_fd = adopt_fd(authority_raw)?;
        let guest_runtime_fd = adopt_fd(guest_runtime_raw)?;
        crate::sys::validate_realm_child_fd(listener.as_fd(), RealmChildFdKind::BrokerListener)
            .map_err(RunError::Io)?;
        crate::sys::validate_realm_child_fd(bootstrap.as_fd(), RealmChildFdKind::BootstrapSession)
            .map_err(RunError::Io)?;
        crate::sys::validate_realm_child_fd(cgroup.as_fd(), RealmChildFdKind::CgroupLeaf)
            .map_err(RunError::Io)?;
        crate::sys::validate_realm_child_fd(authority_fd.as_fd(), RealmChildFdKind::Resource)
            .map_err(RunError::Io)?;
        crate::sys::validate_realm_child_fd(guest_runtime_fd.as_fd(), RealmChildFdKind::Resource)
            .map_err(RunError::Io)?;
        let authority = load_authority(authority_fd)?;
        let (guest_runtime, guest_runtime_digest) = load_guest_runtime(guest_runtime_fd)?;
        if rustix::process::geteuid().as_raw() != 0 || rustix::process::getegid().as_raw() != 0 {
            return Err(protocol("child realm broker namespace credentials invalid"));
        }
        let uid_map = load_id_map("/proc/self/uid_map")?;
        let gid_map = load_id_map("/proc/self/gid_map")?;
        uid_map.verify_namespace_root(authority.broker_uid)?;
        gid_map.verify_namespace_root(authority.broker_gid)?;
        let controller_namespace_uid = uid_map
            .namespace_id_for_host(authority.controller_uid)
            .ok_or_else(|| protocol("child realm broker controller uid unmapped"))?;
        let controller_namespace_gid = gid_map
            .namespace_id_for_host(authority.controller_gid)
            .ok_or_else(|| protocol("child realm broker controller gid unmapped"))?;
        if authority.realm_id != env_value(REALM_ENV)?
            || authority.controller_generation != env_value(GENERATION_ENV)?
            || authority.broker_process_id != env_value(PROCESS_ID_ENV)?
            || authority.session_generation != env_u64(SESSION_GENERATION_ENV)?
            || authority.cgroup_digest != env_digest(CGROUP_DIGEST_ENV)?
            || authority.guest_runtime_digest != guest_runtime_digest
            || guest_runtime.realm_id != authority.realm_id
            || guest_runtime.session_generation != authority.session_generation
        {
            return Err(protocol("child realm broker authority mismatch"));
        }
        Ok(Self {
            listener,
            bootstrap,
            _cgroup: cgroup,
            authority,
            guest_runtime,
            controller_namespace_uid,
            controller_namespace_gid,
        })
    }
}

pub fn run_child_realm_broker(config: ChildRealmBrokerConfig) -> Result<(), RunError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(RunError::Io)?;
    runtime.block_on(serve_child_realm_broker(config))
}

async fn serve_child_realm_broker(config: ChildRealmBrokerConfig) -> Result<(), RunError> {
    let binding = Arc::new(
        RealmBrokerSessionBinding::new_namespace_mapped(
            config.authority.realm_id.clone(),
            config.controller_namespace_uid,
            config.controller_namespace_gid,
            config.authority.session_generation,
        )
        .map_err(|_| protocol("child realm broker binding invalid"))?,
    );
    let handler = Arc::new(build_guest_material_handler(
        &config.authority,
        config.guest_runtime,
    )?);
    let listener = tokio::io::unix::AsyncFd::new(config.listener).map_err(RunError::Io)?;
    send(
        config.bootstrap.as_raw_fd(),
        b"ready",
        MsgFlags::MSG_NOSIGNAL,
    )
    .map_err(|error| RunError::Io(error.into()))?;
    loop {
        let mut ready = listener.readable().await.map_err(RunError::Io)?;
        let accepted = match ready.try_io(|inner| {
            accept4(
                inner.get_ref().as_raw_fd(),
                SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            )
            .map(crate::sys::owned_fd_from_raw)
            .map_err(|error| std::io::Error::from_raw_os_error(error as i32))
        }) {
            Ok(Ok(fd)) => fd,
            Ok(Err(error)) => return Err(RunError::Io(error)),
            Err(_) => continue,
        };
        if let Err(error) = binding.serve(accepted, Arc::clone(&handler)).await {
            tracing::warn!(
                error = %error,
                "child realm broker session closed before service completion"
            );
        }
    }
}

fn load_authority(fd: OwnedFd) -> Result<RealmBrokerChildAuthority, RunError> {
    let status = OFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(|error| RunError::Io(error.into()))?,
    );
    let descriptor = FdFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).map_err(|error| RunError::Io(error.into()))?,
    );
    let seals = SealFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GET_SEALS).map_err(|error| RunError::Io(error.into()))?,
    );
    let required = SealFlag::F_SEAL_WRITE
        | SealFlag::F_SEAL_GROW
        | SealFlag::F_SEAL_SHRINK
        | SealFlag::F_SEAL_SEAL;
    if status & OFlag::O_ACCMODE != OFlag::O_RDONLY
        || !descriptor.contains(FdFlag::FD_CLOEXEC)
        || !seals.contains(required)
    {
        return Err(protocol("child realm broker authority fd unsafe"));
    }
    let mut encoded = Vec::new();
    File::from(fd)
        .take(MAX_AUTHORITY_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(RunError::Io)?;
    if encoded.is_empty() || encoded.len() as u64 > MAX_AUTHORITY_BYTES {
        return Err(protocol("child realm broker authority size invalid"));
    }
    RealmBrokerChildAuthority::decode(&encoded)
        .map_err(|_| protocol("child realm broker authority invalid"))
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
    fn parse(encoded: &str) -> Result<Self, RunError> {
        let mut entries = Vec::new();
        for line in encoded.lines() {
            let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(protocol("child realm broker id map invalid"));
            }
            let parse = |field: &str| {
                field
                    .parse::<u32>()
                    .ok()
                    .ok_or_else(|| protocol("child realm broker id map invalid"))
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
                return Err(protocol("child realm broker id map invalid"));
            }
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(protocol("child realm broker id map invalid"));
        }
        Ok(Self { entries })
    }

    fn verify_namespace_root(&self, expected_host_id: u32) -> Result<(), RunError> {
        if self.entries.iter().any(|entry| {
            entry.namespace_start == 0 && entry.host_start == expected_host_id && entry.length == 1
        }) {
            Ok(())
        } else {
            Err(protocol(
                "child realm broker namespace root mapping mismatch",
            ))
        }
    }

    fn namespace_id_for_host(&self, host_id: u32) -> Option<u32> {
        self.entries.iter().find_map(|entry| {
            let offset = host_id.checked_sub(entry.host_start)?;
            (offset < entry.length).then(|| entry.namespace_start + offset)
        })
    }
}

fn ranges_overlap(left_start: u32, left_len: u32, right_start: u32, right_len: u32) -> bool {
    left_start < right_start + right_len && right_start < left_start + left_len
}

fn load_id_map(path: &'static str) -> Result<IdMap, RunError> {
    let mut encoded = String::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_ID_MAP_BYTES + 1)
                .read_to_string(&mut encoded)
                .map(|_| ())
        })
        .map_err(RunError::Io)?;
    if encoded.is_empty() || encoded.len() as u64 > MAX_ID_MAP_BYTES {
        return Err(protocol("child realm broker id map size invalid"));
    }
    IdMap::parse(&encoded)
}

fn load_guest_runtime(
    fd: OwnedFd,
) -> Result<(RealmBrokerGuestRuntimeBootstrap, [u8; 32]), RunError> {
    let status = OFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(|error| RunError::Io(error.into()))?,
    );
    let descriptor = FdFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).map_err(|error| RunError::Io(error.into()))?,
    );
    let seals = SealFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GET_SEALS).map_err(|error| RunError::Io(error.into()))?,
    );
    let required = SealFlag::F_SEAL_WRITE
        | SealFlag::F_SEAL_GROW
        | SealFlag::F_SEAL_SHRINK
        | SealFlag::F_SEAL_SEAL;
    if status & OFlag::O_ACCMODE != OFlag::O_RDONLY
        || !descriptor.contains(FdFlag::FD_CLOEXEC)
        || !seals.contains(required)
    {
        return Err(protocol("child realm broker guest runtime fd unsafe"));
    }
    let mut encoded = Zeroizing::new(Vec::new());
    File::from(fd)
        .take(MAX_GUEST_RUNTIME_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(RunError::Io)?;
    if encoded.is_empty() || encoded.len() as u64 > MAX_GUEST_RUNTIME_BYTES {
        return Err(protocol("child realm broker guest runtime size invalid"));
    }
    let digest = Sha256::digest(&encoded).into();
    let runtime = RealmBrokerGuestRuntimeBootstrap::decode(&encoded)
        .map_err(|_| protocol("child realm broker guest runtime invalid"))?;
    Ok((runtime, digest))
}

fn adopt_fd(raw: RawFd) -> Result<OwnedFd, RunError> {
    let inherited = crate::sys::owned_fd_from_raw(raw);
    rustix::io::fcntl_dupfd_cloexec(&inherited, 3).map_err(|error| RunError::Io(error.into()))
}

fn env_fd(name: &str) -> Result<RawFd, RunError> {
    env_value(name)?
        .parse::<RawFd>()
        .ok()
        .filter(|fd| (3..=4096).contains(fd))
        .ok_or_else(|| protocol("child realm broker fd invalid"))
}

fn env_u64(name: &str) -> Result<u64, RunError> {
    env_value(name)?
        .parse::<u64>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| protocol("child realm broker generation invalid"))
}

fn verify_resource_binding(fd: RawFd, expected_id: &str) -> Result<(), RunError> {
    let expected_fd = fd.to_string();
    let matched = (0..128).any(|index| {
        let fd_key = format!("D2B_RESOURCE_FD_{index}");
        let id_key = format!("{fd_key}_ID");
        env::var(&fd_key).ok().as_deref() == Some(expected_fd.as_str())
            && env::var(&id_key).ok().as_deref() == Some(expected_id)
    });
    if matched {
        Ok(())
    } else {
        Err(protocol("child realm broker resource binding missing"))
    }
}

fn env_digest(name: &str) -> Result<[u8; 32], RunError> {
    let encoded = env_value(name)?;
    if encoded.len() != 64 {
        return Err(protocol("child realm broker digest invalid"));
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
            .map_err(|_| protocol("child realm broker digest invalid"))?;
    }
    Ok(digest)
}

fn env_value(name: &str) -> Result<String, RunError> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| protocol("child realm broker environment invalid"))
}

fn protocol(message: &'static str) -> RunError {
    RunError::Protocol(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_maps_bind_namespace_root_and_translate_controller_identity() {
        let map = IdMap::parse("0 1001 1\n1 1000 1\n").unwrap();
        map.verify_namespace_root(1001).unwrap();
        assert_eq!(map.namespace_id_for_host(1001), Some(0));
        assert_eq!(map.namespace_id_for_host(1000), Some(1));
        assert_eq!(map.namespace_id_for_host(2000), None);
    }

    #[test]
    fn id_maps_reject_wrong_root_range_and_overlap() {
        assert!(
            IdMap::parse("0 1001 2\n")
                .unwrap()
                .verify_namespace_root(1001)
                .is_err()
        );
        assert!(IdMap::parse("0 1001 1\n0 1002 1\n").is_err());
        assert!(IdMap::parse("0 1001 1\n1 1001 1\n").is_err());
        assert!(IdMap::parse("0 nope 1\n").is_err());
    }
}
