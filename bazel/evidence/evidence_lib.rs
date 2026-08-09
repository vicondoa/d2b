//! Genuine rules_rust evidence.
//!
//! ```
//! assert_eq!(d2b_rules_rust_evidence::identity(7), 7);
//! ```

/// Returns its input.
pub const fn identity(value: u32) -> u32 {
    value
}

#[cfg(test)]
mod tests {
    use super::identity;

    #[test]
    fn identity_is_stable() {
        assert_eq!(identity(7), 7);
    }
}
