use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const MAX_MANAGER_ENVIRONMENT_ENTRIES: usize = 4096;
pub const MAX_MANAGER_ENVIRONMENT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentError {
    TooManyEntries,
    TooLarge,
    InvalidEntry,
    DuplicateKey,
    PathMissing,
    RuntimeDirectoryInvalid,
    ExecutableUnavailable,
    ProxyUnavailable,
    WaylandUnavailable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManagerEnvironment {
    entries: BTreeMap<String, String>,
    encoded_bytes: usize,
}

impl fmt::Debug for ManagerEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagerEnvironment")
            .field("entry_count", &self.entries.len())
            .field("encoded_bytes", &self.encoded_bytes)
            .finish()
    }
}

impl ManagerEnvironment {
    pub fn parse(raw: Vec<String>) -> Result<Self, EnvironmentError> {
        if raw.len() > MAX_MANAGER_ENVIRONMENT_ENTRIES {
            return Err(EnvironmentError::TooManyEntries);
        }
        let encoded_bytes = raw.iter().try_fold(0usize, |total, entry| {
            total
                .checked_add(entry.len())
                .and_then(|value| value.checked_add(1))
                .ok_or(EnvironmentError::TooLarge)
        })?;
        if encoded_bytes > MAX_MANAGER_ENVIRONMENT_BYTES {
            return Err(EnvironmentError::TooLarge);
        }

        let mut entries = BTreeMap::new();
        for entry in raw {
            let (key, value) = entry
                .split_once('=')
                .ok_or(EnvironmentError::InvalidEntry)?;
            if !valid_key(key) || value.contains('\0') {
                return Err(EnvironmentError::InvalidEntry);
            }
            if entries.insert(key.to_owned(), value.to_owned()).is_some() {
                return Err(EnvironmentError::DuplicateKey);
            }
        }
        Ok(Self {
            entries,
            encoded_bytes,
        })
    }

    pub fn child_entries(
        &self,
        graphical: bool,
        proxy_wayland_display: Option<&str>,
    ) -> Result<BTreeMap<String, String>, EnvironmentError> {
        let mut entries = self.entries.clone();
        if graphical {
            let display = proxy_wayland_display.ok_or(EnvironmentError::ProxyUnavailable)?;
            if !valid_proxy_display(display) {
                return Err(EnvironmentError::ProxyUnavailable);
            }
            entries.remove("DISPLAY");
            entries.insert("WAYLAND_DISPLAY".to_owned(), display.to_owned());
        }
        Ok(entries)
    }

    pub fn path(&self) -> Result<&str, EnvironmentError> {
        self.entries
            .get("PATH")
            .map(String::as_str)
            .filter(|path| !path.is_empty())
            .ok_or(EnvironmentError::PathMissing)
    }

    pub fn state_home(&self, passwd_home: &Path) -> PathBuf {
        self.entries
            .get("XDG_STATE_HOME")
            .filter(|value| value.starts_with('/') && !value.contains('\0'))
            .map(PathBuf::from)
            .unwrap_or_else(|| passwd_home.join(".local/state"))
    }

    pub fn runtime_directory(&self) -> Result<PathBuf, EnvironmentError> {
        self.entries
            .get("XDG_RUNTIME_DIR")
            .filter(|value| value.starts_with('/') && !value.contains('\0'))
            .map(PathBuf::from)
            .ok_or(EnvironmentError::RuntimeDirectoryInvalid)
    }

    pub fn wayland_display(&self) -> Result<&str, EnvironmentError> {
        self.entries
            .get("WAYLAND_DISPLAY")
            .map(String::as_str)
            .filter(|value| {
                !value.is_empty()
                    && !value.contains('\0')
                    && !value.split('/').any(|component| component == "..")
            })
            .ok_or(EnvironmentError::WaylandUnavailable)
    }

    pub fn resolve_program(&self, program: &str) -> Result<PathBuf, EnvironmentError> {
        if program.is_empty() || program.contains('\0') {
            return Err(EnvironmentError::ExecutableUnavailable);
        }

        if program.contains('/') {
            let path = PathBuf::from(program);
            return executable_file(&path)
                .then_some(path)
                .ok_or(EnvironmentError::ExecutableUnavailable);
        }

        let mut seen = BTreeSet::new();
        for directory in self.path()?.split(':') {
            if directory.is_empty() || !directory.starts_with('/') || !seen.insert(directory) {
                continue;
            }
            let candidate = Path::new(directory).join(program);
            if executable_file(&candidate) {
                return Ok(candidate);
            }
        }
        Err(EnvironmentError::ExecutableUnavailable)
    }
}

pub(crate) fn valid_proxy_display(display: &str) -> bool {
    let Some((directory, socket)) = display.split_once('/') else {
        return false;
    };
    socket == "wayland.sock"
        && !directory.contains('/')
        && directory
            .strip_prefix("d2b-unsafe-local-")
            .is_some_and(|suffix| {
                suffix.len() == 32
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
}

fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(first) if first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_environment_is_complete_and_debug_redacted() {
        let canary = "environment-value-canary";
        let environment = ManagerEnvironment::parse(vec![
            "PATH=/run/current-system/sw/bin".to_owned(),
            format!("PRIVATE_VALUE={canary}"),
            "DISPLAY=:0".to_owned(),
            "WAYLAND_DISPLAY=wayland-1".to_owned(),
        ])
        .unwrap();
        let debug = format!("{environment:?}");
        assert!(!debug.contains(canary));
        assert!(!debug.contains("PRIVATE_VALUE"));

        let child = environment.child_entries(false, None).unwrap();
        assert_eq!(child.get("PRIVATE_VALUE").map(String::as_str), Some(canary));
        assert_eq!(child.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(
            child.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-1")
        );
    }

    #[test]
    fn graphical_environment_never_falls_back_to_real_display() {
        let environment = ManagerEnvironment::parse(vec![
            "PATH=/bin".to_owned(),
            "DISPLAY=:0".to_owned(),
            "WAYLAND_DISPLAY=wayland-real".to_owned(),
        ])
        .unwrap();
        assert_eq!(
            environment.child_entries(true, None),
            Err(EnvironmentError::ProxyUnavailable)
        );
        let child = environment
            .child_entries(
                true,
                Some("d2b-unsafe-local-00112233445566778899aabbccddeeff/wayland.sock"),
            )
            .unwrap();
        assert!(!child.contains_key("DISPLAY"));
        assert_eq!(
            child.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("d2b-unsafe-local-00112233445566778899aabbccddeeff/wayland.sock")
        );
        for invalid in [
            "wayland.sock",
            "/absolute/wayland.sock",
            "../wayland.sock",
            "d2b-unsafe-local-00112233445566778899aabbccddeeff/../wayland.sock",
            "d2b-unsafe-local-00112233445566778899aabbccddeefg/wayland.sock",
            "d2b-unsafe-local-00112233445566778899aabbccddeeff/other.sock",
        ] {
            assert_eq!(
                environment.child_entries(true, Some(invalid)),
                Err(EnvironmentError::ProxyUnavailable),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn wayland_display_accepts_socket_basename_and_rejects_invalid_values() {
        let with_display = |display: Option<&str>| ManagerEnvironment {
            entries: display
                .map(|value| BTreeMap::from([("WAYLAND_DISPLAY".to_owned(), value.to_owned())]))
                .unwrap_or_default(),
            encoded_bytes: 0,
        };

        assert_eq!(
            with_display(Some("wayland-1")).wayland_display(),
            Ok("wayland-1")
        );
        for display in [None, Some(""), Some("wayland\0-1"), Some("../wayland-1")] {
            assert_eq!(
                with_display(display).wayland_display(),
                Err(EnvironmentError::WaylandUnavailable)
            );
        }
    }

    #[test]
    fn malformed_or_ambiguous_environment_fails_closed() {
        assert_eq!(
            ManagerEnvironment::parse(vec!["NO_EQUALS".to_owned()]),
            Err(EnvironmentError::InvalidEntry)
        );
        assert_eq!(
            ManagerEnvironment::parse(vec!["A=1".to_owned(), "A=2".to_owned()]),
            Err(EnvironmentError::DuplicateKey)
        );
        assert_eq!(
            ManagerEnvironment::parse(vec!["1BAD=value".to_owned()]),
            Err(EnvironmentError::InvalidEntry)
        );
    }
}
