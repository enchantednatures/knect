//! Tap callback tokens.
//!
//! Tokens are random 128-bit values rendered as 32 lower-case hex characters.
//! Comparison never branches on the token contents and always inspects a fixed
//! number of bytes, so a mismatched length cannot short-circuit the check.

use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Length of every generated token in bytes.
pub(crate) const TOKEN_LEN: usize = 32;

/// Generates a fresh callback token.
pub(crate) fn generate_token() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Compares an expected token with a caller-supplied token in constant time.
pub(crate) fn tokens_match(expected: &str, provided: &str) -> bool {
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();

    let mut left = [0_u8; TOKEN_LEN];
    let mut right = [0_u8; TOKEN_LEN];

    let expected_len = expected.len().min(TOKEN_LEN);
    let provided_len = provided.len().min(TOKEN_LEN);
    left[..expected_len].copy_from_slice(&expected[..expected_len]);
    right[..provided_len].copy_from_slice(&provided[..provided_len]);

    let bytes_equal = left.ct_eq(&right);
    let lengths_equal = (expected.len() as u64).ct_eq(&(provided.len() as u64));
    bool::from(bytes_equal & lengths_equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_accept_only_an_exact_match() {
        let token = generate_token();
        assert_eq!(token.len(), TOKEN_LEN);
        assert!(tokens_match(&token, &token));
        assert!(!tokens_match(&token, ""));
        assert!(!tokens_match(&token, &token[..TOKEN_LEN - 1]));
        assert!(!tokens_match(&token, &format!("{token}0")));
        assert!(!tokens_match(&token, &"0".repeat(TOKEN_LEN)));
    }

    #[test]
    fn generated_tokens_are_distinct() {
        assert_ne!(generate_token(), generate_token());
    }
}
