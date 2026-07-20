use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Clone, PartialEq, Eq)]
pub struct ExternalDataPlaneSocket(PathBuf);

impl ExternalDataPlaneSocket {
    pub fn new(path: PathBuf) -> Result<Self> {
        validate_external_socket_path(&path)?;
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

pub fn validate_external_socket_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("libshpool socket path must not be empty");
    }
    if !path.is_absolute() {
        bail!("libshpool socket path must be absolute");
    }
    if path.as_os_str().as_encoded_bytes().len() >= 108 {
        bail!("libshpool socket path is too long for sockaddr_un");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ExternalDataPlaneSocket, validate_external_socket_path};

    #[test]
    fn rejects_empty_paths() {
        let err = validate_external_socket_path(Path::new(""))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn rejects_relative_paths() {
        let err = validate_external_socket_path(Path::new("relative.sock"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must be absolute"), "{err}");
    }

    #[test]
    fn rejects_paths_too_long_for_sockaddr_un() {
        let path = format!("/run/user/1000/{}", "a".repeat(120));
        let err = validate_external_socket_path(Path::new(&path))
            .unwrap_err()
            .to_string();
        assert!(err.contains("sockaddr_un"), "{err}");
    }

    #[test]
    fn wrapper_exposes_only_valid_external_paths() {
        let socket =
            ExternalDataPlaneSocket::new(Path::new("/run/user/1000/d2b-shpool.sock").into())
                .unwrap();
        assert_eq!(
            socket.as_path(),
            Path::new("/run/user/1000/d2b-shpool.sock")
        );
    }
}
