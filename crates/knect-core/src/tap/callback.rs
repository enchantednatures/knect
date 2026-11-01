//! CloudEvent parsing, filtering, and header redaction for tap callbacks.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use knect_proto::redaction::REDACTED;
use knect_proto::tap::{TapEvent, TapFilter};
use serde_json::Value;
use uuid::Uuid;

use crate::error::CoreError;

/// Maximum accepted callback body size.
pub(crate) const MAX_BODY_BYTES: usize = 1_048_576;
/// Maximum accepted callback header count.
pub(crate) const MAX_HEADERS: usize = 64;
/// Maximum retained length of one callback header value.
pub(crate) const MAX_HEADER_VALUE: usize = 8_192;

/// One callback delivery handed to a tap manager.
pub struct TapCallbackRequest {
    /// HTTP headers with lower-case names.
    pub headers: BTreeMap<String, String>,
    /// Raw request body.
    pub body: Vec<u8>,
    /// Time the daemon received the request.
    pub received_at: DateTime<Utc>,
}

/// Result of an authenticated callback delivery.
#[derive(Debug, Clone, PartialEq)]
pub enum TapDelivery {
    /// The event matched the filter and was retained.
    Published(TapEvent),
    /// The event was authenticated but filtered out.
    Filtered,
}

/// Parses a CloudEvent in binary or structured mode.
///
/// Returns `None` when the event does not satisfy the tap filter.
pub(crate) fn parse_cloud_event(
    request: TapCallbackRequest,
    filter: Option<&TapFilter>,
) -> Result<Option<TapEvent>, CoreError> {
    if request.body.len() > MAX_BODY_BYTES {
        return Err(invalid(format!(
            "body exceeds {MAX_BODY_BYTES} bytes ({} bytes)",
            request.body.len()
        )));
    }
    if request.headers.len() > MAX_HEADERS {
        return Err(invalid(format!(
            "request has {} headers; the limit is {MAX_HEADERS}",
            request.headers.len()
        )));
    }

    let headers = redact_headers(&request.headers);
    let parsed = if is_structured(&headers, &request.body)? {
        parse_structured(&headers, &request.body)?
    } else {
        parse_binary(&headers, &request.body)?
    };

    if !matches_filter(filter, &parsed.attributes) {
        return Ok(None);
    }

    Ok(Some(TapEvent {
        id: format!("evt-{}", Uuid::new_v4().simple()),
        ce_id: parsed.ce_id,
        ce_source: parsed.ce_source,
        ce_type: parsed.ce_type,
        ce_subject: parsed.ce_subject,
        ce_time: parsed.ce_time,
        headers,
        payload: parsed.payload,
        received_at: request.received_at,
    }))
}

/// Redacts credential-bearing headers while preserving diagnostic headers.
#[must_use]
pub fn redact_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            let value = if sensitive_header(&name) {
                REDACTED.to_owned()
            } else {
                value.chars().take(MAX_HEADER_VALUE).collect()
            };
            (name, value)
        })
        .collect()
}

struct ParsedEvent {
    ce_id: String,
    ce_source: String,
    ce_type: String,
    ce_subject: Option<String>,
    ce_time: Option<DateTime<Utc>>,
    payload: Value,
    attributes: BTreeMap<String, String>,
}

fn is_structured(headers: &BTreeMap<String, String>, body: &[u8]) -> Result<bool, CoreError> {
    if media_type(headers).is_empty() {
        return Ok(false);
    }
    if media_type(headers) == "application/cloudevents+json" {
        return Ok(true);
    }
    if headers.contains_key("ce-id") {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Ok(false);
    };
    Ok(value
        .as_object()
        .is_some_and(|object| object.contains_key("specversion")))
}

fn parse_binary(headers: &BTreeMap<String, String>, body: &[u8]) -> Result<ParsedEvent, CoreError> {
    let specversion = required(headers, "ce-specversion")?;
    if specversion != "1.0" {
        return Err(invalid(format!(
            "unsupported ce-specversion '{specversion}'"
        )));
    }
    let ce_id = required(headers, "ce-id")?.to_owned();
    let ce_source = required(headers, "ce-source")?.to_owned();
    let ce_type = required(headers, "ce-type")?.to_owned();
    let ce_subject = headers.get("ce-subject").cloned();
    let ce_time = headers
        .get("ce-time")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    let mut attributes = BTreeMap::new();
    attributes.insert("specversion".to_owned(), specversion.to_owned());
    attributes.insert("id".to_owned(), ce_id.clone());
    attributes.insert("source".to_owned(), ce_source.clone());
    attributes.insert("type".to_owned(), ce_type.clone());
    if let Some(subject) = &ce_subject {
        attributes.insert("subject".to_owned(), subject.clone());
    }
    if let Some(time) = &ce_time {
        attributes.insert("time".to_owned(), time.to_rfc3339());
    }
    for (name, value) in headers {
        if let Some(extension) = name.strip_prefix("ce-")
            && !matches!(
                extension,
                "specversion" | "id" | "source" | "type" | "subject" | "time"
            )
        {
            attributes.insert(extension.to_owned(), value.clone());
        }
    }

    let payload = parse_payload(media_type(headers), body)?;
    Ok(ParsedEvent {
        ce_id,
        ce_source,
        ce_type,
        ce_subject,
        ce_time,
        payload,
        attributes,
    })
}

fn parse_structured(
    _headers: &BTreeMap<String, String>,
    body: &[u8],
) -> Result<ParsedEvent, CoreError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| invalid(format!("structured body is not JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("structured body must be a JSON object"))?;

    let specversion = string_field(object, "specversion")?;
    if specversion != "1.0" {
        return Err(invalid(format!("unsupported specversion '{specversion}'")));
    }
    let ce_id = string_field(object, "id")?;
    let ce_source = string_field(object, "source")?;
    let ce_type = string_field(object, "type")?;
    let ce_subject = object
        .get("subject")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let ce_time = object
        .get("time")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    let payload = if let Some(encoded) = object.get("data_base64").and_then(Value::as_str) {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|error| invalid(format!("data_base64 is not valid base64: {error}")))?;
        Value::String(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        object.get("data").cloned().unwrap_or(Value::Null)
    };

    let mut attributes = BTreeMap::new();
    for (key, value) in object {
        if matches!(key.as_str(), "data" | "data_base64") {
            continue;
        }
        if let Some(text) = value.as_str() {
            attributes.insert(key.to_ascii_lowercase(), text.to_owned());
        }
    }

    Ok(ParsedEvent {
        ce_id,
        ce_source,
        ce_type,
        ce_subject,
        ce_time,
        payload,
        attributes,
    })
}

fn parse_payload(media_type: &str, body: &[u8]) -> Result<Value, CoreError> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    if media_type == "application/json" || media_type.ends_with("+json") {
        return serde_json::from_slice(body)
            .map_err(|error| invalid(format!("binary data is not JSON: {error}")));
    }
    Ok(Value::String(String::from_utf8_lossy(body).into_owned()))
}

fn media_type(headers: &BTreeMap<String, String>) -> &str {
    headers
        .get("content-type")
        .map_or("", |value| value.split(';').next().map_or("", str::trim))
}

fn required<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, CoreError> {
    headers
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| invalid(format!("missing {name} header")))
}

fn string_field(object: &serde_json::Map<String, Value>, name: &str) -> Result<String, CoreError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(format!("missing or non-string '{name}'")))
}

fn matches_filter(filter: Option<&TapFilter>, attributes: &BTreeMap<String, String>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    filter.attributes.iter().all(|(key, expected)| {
        attributes
            .get(&key.to_ascii_lowercase())
            .is_some_and(|actual| actual == expected)
    })
}

fn sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "x-knect-tap-token"
            | "x-csrf-token"
    ) || name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("api-key")
        || name.contains("apikey")
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::Tap(format!(
        "tap callback invalid CloudEvent: {}",
        message.into()
    ))
}
