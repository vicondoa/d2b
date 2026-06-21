use thiserror::Error;

pub const MAX_SHELL_NAME_BYTES: usize = 64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShellNameError {
    #[error("shell session name must not be empty")]
    Empty,
    #[error("shell session name must be at most {MAX_SHELL_NAME_BYTES} bytes")]
    TooLong,
    #[error("shell session name must not be '.' or '..'")]
    DotName,
    #[error("shell session name contains unsupported character '{0}'")]
    UnsupportedChar(char),
    #[error("shell session name must start with an ASCII letter, digit, or '_'")]
    BadFirstChar,
}

pub fn validate_shell_name(name: &str) -> Result<(), ShellNameError> {
    if name.is_empty() {
        return Err(ShellNameError::Empty);
    }
    if name.len() > MAX_SHELL_NAME_BYTES {
        return Err(ShellNameError::TooLong);
    }
    if name == "." || name == ".." {
        return Err(ShellNameError::DotName);
    }

    let mut chars = name.chars();
    let first = chars.next().expect("non-empty checked above");
    if !first.is_ascii_alphanumeric() && first != '_' {
        return Err(ShellNameError::BadFirstChar);
    }

    for ch in chars {
        if !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '-') {
            return Err(ShellNameError::UnsupportedChar(ch));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ShellNameError, validate_shell_name};

    #[test]
    fn accepts_plain_names() {
        for name in ["default", "work-1", "work.dev", "_scratch", "A9"] {
            validate_shell_name(name).unwrap();
        }
    }

    #[test]
    fn rejects_option_and_path_confusion() {
        assert_eq!(
            validate_shell_name("-default"),
            Err(ShellNameError::BadFirstChar)
        );
        assert_eq!(
            validate_shell_name("/default"),
            Err(ShellNameError::BadFirstChar)
        );
        assert_eq!(
            validate_shell_name("work/dev"),
            Err(ShellNameError::UnsupportedChar('/'))
        );
    }

    #[test]
    fn rejects_shpool_template_syntax() {
        assert_eq!(
            validate_shell_name("{workspace}"),
            Err(ShellNameError::BadFirstChar)
        );
        assert_eq!(
            validate_shell_name("work-{workspace}"),
            Err(ShellNameError::UnsupportedChar('{'))
        );
        assert_eq!(
            validate_shell_name("work-$USER"),
            Err(ShellNameError::UnsupportedChar('$'))
        );
    }

    #[test]
    fn rejects_empty_dot_and_too_long() {
        assert_eq!(validate_shell_name(""), Err(ShellNameError::Empty));
        assert_eq!(validate_shell_name("."), Err(ShellNameError::DotName));
        assert_eq!(validate_shell_name(".."), Err(ShellNameError::DotName));
        assert_eq!(
            validate_shell_name(&"a".repeat(65)),
            Err(ShellNameError::TooLong)
        );
    }
}
