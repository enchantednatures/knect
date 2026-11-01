//! SSE and JSONL event decoding helpers.
//!
//! The HTTP transport delivers `text/event-stream` frames; the Unix transport
//! delivers one `SseEvent` JSON object per line. Both funnel through the
//! parsers here so the app only ever sees typed [`SseEvent`] values.

use knect_proto::SseEvent;

use super::ClientError;

/// One decoded SSE frame before the JSON payload is parsed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// `id:` field, when present.
    pub id: Option<u64>,
    /// `event:` field, when present.
    pub event: Option<String>,
    /// Concatenated `data:` fields.
    pub data: String,
}

/// Removes the first complete SSE frame from `buffer`.
///
/// Frames end at a blank line (`\n\n` or `\r\n\r\n`). Returns `None` when the
/// buffer does not yet contain a full frame, so callers can keep appending
/// network chunks.
pub fn take_frame(buffer: &mut String) -> Option<String> {
    let (index, separator_len) = frame_boundary(buffer)?;
    let frame = buffer.get(..index)?.to_owned();
    buffer.drain(..index + separator_len);
    Some(frame)
}

fn frame_boundary(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

/// Parses one SSE frame into its fields.
#[must_use]
pub fn parse_frame(raw: &str) -> SseFrame {
    let mut frame = SseFrame::default();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "id" => frame.id = value.trim().parse::<u64>().ok(),
            "event" => frame.event = Some(value.to_owned()),
            "data" => {
                if !frame.data.is_empty() {
                    frame.data.push('\n');
                }
                frame.data.push_str(value);
            }
            _ => {}
        }
    }
    frame
}

/// Parses an SSE frame into a typed event.
///
/// Returns `Ok(None)` for comment-only or heartbeat frames without data.
pub fn decode_frame(raw: &str) -> Result<Option<SseEvent>, ClientError> {
    let frame = parse_frame(raw);
    if frame.data.trim().is_empty() {
        return Ok(None);
    }
    let event: SseEvent = serde_json::from_str(frame.data.trim())
        .map_err(|error| ClientError::Protocol(format!("invalid SSE event payload: {error}")))?;
    if let Some(name) = frame.event
        && name != event.event_type()
    {
        return Err(ClientError::Protocol(format!(
            "SSE event name '{name}' does not match payload '{}'",
            event.event_type()
        )));
    }
    if let Some(id) = frame.id
        && id != event.seq
    {
        return Err(ClientError::Protocol(format!(
            "SSE frame id {id} does not match event sequence {}",
            event.seq
        )));
    }
    Ok(Some(event))
}

/// Decodes one newline-delimited JSON event line.
pub fn decode_jsonl_event(line: &str) -> Result<Option<SseEvent>, ClientError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let event: SseEvent = serde_json::from_str(trimmed)
        .map_err(|error| ClientError::Protocol(format!("invalid JSONL event: {error}")))?;
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::events::{Heartbeat, SseEventKind};

    fn heartbeat(seq: u64) -> SseEvent {
        SseEvent::new(
            seq,
            Utc::now(),
            SseEventKind::Heartbeat {
                data: Heartbeat {
                    uptime_secs: seq,
                    generation: seq,
                    node_count: 1,
                    edge_count: 0,
                    ts: Utc::now(),
                },
            },
        )
    }

    #[test]
    fn frames_are_split_across_arbitrary_chunks() {
        let event = heartbeat(9);
        let payload = serde_json::to_string(&event).expect("encode");
        let mut buffer = format!("id: 9\nevent: daemon.heartbeat\ndata: {payload}\n\n");
        assert!(take_frame(&mut buffer).is_some());
        assert!(buffer.is_empty());

        buffer.push_str(&format!("id: 10\nevent: daemon.heartbeat\ndata: {payload}"));
        assert!(take_frame(&mut buffer).is_none());
        buffer.push_str("\n\n");
        let frame = take_frame(&mut buffer).expect("complete frame");
        assert!(frame.contains("id: 10"));
    }

    #[test]
    fn parse_frame_handles_comments_and_multiline_data() {
        let raw = ": keepalive\nid: 3\nevent: daemon.heartbeat\ndata: {\"seq\":\ndata: 3}\n";
        let frame = parse_frame(raw);
        assert_eq!(frame.id, Some(3));
        assert_eq!(frame.event.as_deref(), Some("daemon.heartbeat"));
        assert_eq!(frame.data, "{\"seq\":\n3}");
    }

    #[test]
    fn decode_frame_rejects_mismatched_names() {
        let event = heartbeat(1);
        let payload = serde_json::to_string(&event).expect("encode");
        let raw = format!("id: 1\nevent: snapshot\ndata: {payload}");
        assert!(matches!(
            decode_frame(&raw),
            Err(ClientError::Protocol(message)) if message.contains("does not match")
        ));
    }

    #[test]
    fn decode_frame_accepts_contract_payload() {
        let event = heartbeat(2);
        let payload = serde_json::to_string(&event).expect("encode");
        let raw = format!("id: 2\nevent: daemon.heartbeat\ndata: {payload}");
        let decoded = decode_frame(&raw).expect("decode").expect("event");
        assert_eq!(decoded.seq, 2);
    }

    #[test]
    fn jsonl_events_ignore_blank_lines() {
        let event = heartbeat(4);
        let payload = serde_json::to_string(&event).expect("encode");
        assert!(decode_jsonl_event("  ").expect("blank").is_none());
        let decoded = decode_jsonl_event(&payload)
            .expect("decode")
            .expect("event");
        assert_eq!(decoded.event_type(), "daemon.heartbeat");
    }
}
