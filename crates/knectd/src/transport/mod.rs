//! Daemon transport layer.
//!
//! HTTP and Unix-domain-socket listeners share the same axum router. The UDS
//! listener peeks the first byte to support newline-delimited JSON-RPC as well
//! as HTTP/1.1, per `docs/protocol.md`.

pub mod http;
pub mod jsonl;
pub mod sse;
pub mod uds;
