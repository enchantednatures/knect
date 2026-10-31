//! Secret handling helpers.
//!
//! [`SecretValue`] is intentionally not serializable. Consumers must pass it
//! through a redactor and store only non-secret evidence in protocol types.

use std::fmt;

/// Placeholder written by [`SecretValue`]'s `Debug` and `Display` impls.
pub const REDACTED: &str = "[REDACTED]";

/// A secret value that never appears in formatting output.
///
/// This type deliberately does **not** implement `Serialize` or
/// `Deserialize`. Decode secrets, extract the minimum non-secret evidence, and
/// drop the value at the end of the parse scope.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wraps a raw secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Explicitly exposes the raw secret.
    ///
    /// Call this only inside the in-memory parsing scope that derives redacted
    /// link evidence. Never write the result to logs, JSON, or a cache.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns true when the secret is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of bytes in the secret without exposing its value.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretValue").field(&REDACTED).finish()
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}
