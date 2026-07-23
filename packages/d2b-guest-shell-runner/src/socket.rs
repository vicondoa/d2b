use std::path::Path;

use anyhow::{Result, bail};

pub fn validate_socket_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("socket path must not be empty");
    }
    if !path.is_absolute() {
        bail!("socket path must be absolute: {path:?}");
    }
    if path.as_os_str().as_encoded_bytes().len() >= 108 {
        bail!("socket path is too long for sockaddr_un: {path:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::validate_socket_path;

    #[test]
    fn rejects_empty_paths() {
        let err = validate_socket_path(Path::new("")).unwrap_err().to_string();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn rejects_relative_paths() {
        let err = validate_socket_path(Path::new("relative.sock"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must be absolute"), "{err}");
    }

    #[test]
    fn rejects_paths_too_long_for_sockaddr_un() {
        let path = format!("/run/user/1000/{}", "a".repeat(120));
        let err = validate_socket_path(Path::new(&path))
            .unwrap_err()
            .to_string();
        assert!(err.contains("sockaddr_un"), "{err}");
    }
}
