//! Tracing initialization and secret-bearing key redaction.

use std::io::Write;
use std::sync::OnceLock;

use knect_core::config::Config;
use regex::Regex;
use tracing_subscriber::EnvFilter;

use crate::error::DaemonError;

/// Initializes the global tracing subscriber.
pub fn init(config: &Config) -> Result<(), DaemonError> {
    let filter = EnvFilter::try_new(&config.daemon.log)
        .map_err(|error| DaemonError::Config(format!("invalid log filter: {error}")))?;
    if config.daemon.log_format == "json" {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(RedactingMakeWriter)
            .try_init()
            .map_err(|error| {
                DaemonError::Config(format!("cannot initialize JSON logging: {error}"))
            })
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(RedactingMakeWriter)
            .try_init()
            .map_err(|error| DaemonError::Config(format!("cannot initialize logging: {error}")))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RedactingMakeWriter;

struct RedactingWriter;

impl Write for RedactingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        let redacted = redact_message(&text);
        std::io::stderr().write_all(redacted.as_bytes())?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter
    }
}

const SECRET_JSON_PATTERN: &str =
    r#"(?i)"(token|password|secret|authorization|api[_-]?key|client[_-]?secret)"(\s*:\s*)"[^"]*""#;
const SECRET_BARE_PATTERN: &str = r#"(?i)(token|password|secret|authorization|api[_-]?key|client[_-]?secret)(\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;}\]]+)"#;

/// Redacts values for known secret-bearing keys in free-form messages.
///
/// Structured logs should keep secrets in fields that never reach a formatter;
/// this helper is the last line of defence for error strings and debug lines.
#[must_use]
pub fn redact_message(input: &str) -> String {
    static JSON_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    static BARE_PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    let json = JSON_PATTERN.get_or_init(|| Regex::new(SECRET_JSON_PATTERN).ok());
    let bare = BARE_PATTERN.get_or_init(|| Regex::new(SECRET_BARE_PATTERN).ok());
    let mut output = input.to_owned();
    if let Some(pattern) = json {
        output = pattern
            .replace_all(&output, r#""$1"$2"[REDACTED]""#)
            .into_owned();
    }
    if let Some(pattern) = bare {
        output = pattern
            .replace_all(&output, "${1}${2}[REDACTED]")
            .into_owned();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_bearing_keys_are_redacted() {
        let message = "request failed token=abc123 password: hunter2 client_secret='shh'";
        let redacted = redact_message(message);
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("shh"));
        assert_eq!(redacted.matches("[REDACTED]").count(), 3);
    }

    #[test]
    fn json_style_values_are_redacted() {
        let redacted = redact_message(r#"{"authorization":"Bearer abc.def","token":"xyz"}"#);
        assert!(!redacted.contains("abc.def"));
        assert!(!redacted.contains("xyz"));
    }

    #[test]
    fn harmless_messages_are_unchanged() {
        let message = "topology generation 42 has 10 nodes";
        assert_eq!(redact_message(message), message);
    }
}
