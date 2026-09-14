//! Log and metric redaction helpers.
//!
//! A Provider frequently holds a value that is fine to compare but must
//! never reach a log line, a span attribute, or a metric label: a resource
//! name, a method argument, a digest, a caller-supplied token. Wrapping it
//! makes the redaction structural rather than a rule each call site has to
//! remember.

/// A value whose `Debug` and `Display` render only `<redacted>`.
///
/// The inner value is reachable only through [`Redacted::expose`], so a
/// reviewer can grep for the exact places a sensitive value leaves the
/// wrapper.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wrap a value.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the wrapped value at an explicit, greppable call site.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Unwrap the value at an explicit, greppable call site.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> core::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T> core::fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_value_never_renders_itself() {
        let secret = Redacted::new("provider-agent-argument");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        assert_eq!(format!("{secret}"), "<redacted>");
        assert_eq!(*secret.expose(), "provider-agent-argument");
        assert_eq!(secret.into_inner(), "provider-agent-argument");
    }

    #[test]
    fn a_nested_wrapped_value_never_leaks_through_a_container() {
        let rendered = format!("{:?}", vec![Redacted::new("alpha"), Redacted::new("beta")]);
        assert!(!rendered.contains("alpha"));
        assert!(!rendered.contains("beta"));
    }
}
