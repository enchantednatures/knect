//! SSE event hub, replay ring, and frame encoding.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use axum::response::sse::Event;
use chrono::Utc;
use futures::Stream;
use knect_proto::events::{SseEvent, SseEventKind};
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;

use crate::app::AppState;

/// Result of a replay request.
#[derive(Debug, Clone, PartialEq)]
pub enum Replay {
    /// The client needs a fresh full snapshot.
    Snapshot,
    /// Events after the requested sequence number.
    Events(Vec<SseEvent>),
}

/// Broadcast sender plus bounded replay ring.
#[derive(Debug)]
pub struct EventHub {
    sender: broadcast::Sender<SseEvent>,
    ring: Mutex<VecDeque<SseEvent>>,
    sequence: AtomicU64,
    capacity: usize,
}

impl EventHub {
    /// Creates a hub with a replay ring capacity (clamped to at least one).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            ring: Mutex::new(VecDeque::with_capacity(capacity)),
            sequence: AtomicU64::new(0),
            capacity,
        }
    }

    /// Subscribes to future events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.sender.subscribe()
    }

    /// Returns the underlying broadcast sender.
    #[must_use]
    pub fn sender(&self) -> broadcast::Sender<SseEvent> {
        self.sender.clone()
    }

    /// Reserves the next monotonic sequence number.
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.sequence
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }

    /// Assigns a sequence number and builds an event without broadcasting it.
    #[must_use]
    pub fn next_event(&self, kind: SseEventKind) -> SseEvent {
        SseEvent::new(self.next_sequence(), Utc::now(), kind)
    }

    /// Publishes an event with an already-reserved sequence number.
    ///
    /// This exists so payloads that embed a sequence number (for example
    /// `GraphPatch`) can be built before the envelope and still agree with the
    /// SSE `id` exactly.
    pub fn publish_with_seq(&self, seq: u64, kind: SseEventKind) -> SseEvent {
        let event = SseEvent::new(seq, Utc::now(), kind);
        {
            let mut ring = self.lock_ring();
            if ring.len() == self.capacity {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        let _ = self.sender.send(event.clone());
        event
    }

    /// Publishes an event to the ring and broadcast channel.
    pub fn publish(&self, kind: SseEventKind) -> SseEvent {
        let seq = self.next_sequence();
        self.publish_with_seq(seq, kind)
    }

    /// Returns the replay decision for a client's `Last-Event-ID`.
    #[must_use]
    pub fn replay_after(&self, last_event_id: Option<u64>) -> Replay {
        let Some(last) = last_event_id else {
            return Replay::Snapshot;
        };
        let ring = self.lock_ring();
        let Some(oldest) = ring.front().map(|event| event.seq) else {
            return Replay::Snapshot;
        };
        if last.saturating_add(1) < oldest {
            return Replay::Snapshot;
        }
        Replay::Events(
            ring.iter()
                .filter(|event| event.seq > last)
                .cloned()
                .collect(),
        )
    }

    /// Number of events currently retained for replay.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock_ring().len()
    }

    /// True when no events are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock_ring(&self) -> MutexGuard<'_, VecDeque<SseEvent>> {
        match self.ring.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Formats one protocol SSE frame.
#[must_use]
pub fn format_frame(event: &SseEvent) -> String {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "id: {}\nevent: {}\ndata: {}\n\n",
        event.seq,
        event.event_type(),
        data
    )
}

/// Converts an event to an axum SSE frame.
pub fn encode_event(event: &SseEvent) -> Event {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    Event::default()
        .id(event.seq.to_string())
        .event(event.event_type())
        .data(data)
}

/// Builds the per-client SSE stream.
pub fn event_stream(
    state: AppState,
    last_event_id: Option<u64>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let hub = state.events.clone();
    let heartbeat_interval = state.heartbeat_interval;
    async_stream::stream! {
        let mut receiver = hub.subscribe();
        let mut last_sent = last_event_id.unwrap_or(0);
        match hub.replay_after(last_event_id) {
            Replay::Snapshot => {
                let event = hub.next_event(SseEventKind::Snapshot {
                    data: (*state.topology()).clone(),
                });
                last_sent = event.seq;
                yield Ok(encode_event(&event));
            }
            Replay::Events(events) => {
                for event in events {
                    last_sent = event.seq;
                    yield Ok(encode_event(&event));
                }
            }
        }

        let mut heartbeat = tokio::time::interval(heartbeat_interval);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        heartbeat.tick().await;

        loop {
            tokio::select! {
                received = receiver.recv() => {
                    match received {
                        Ok(event) => {
                            if event.seq > last_sent {
                                last_sent = event.seq;
                                yield Ok(encode_event(&event));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let event = hub.next_event(SseEventKind::Snapshot {
                                data: (*state.topology()).clone(),
                            });
                            last_sent = event.seq;
                            yield Ok(encode_event(&event));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat.tick() => {
                    let event = state.heartbeat_event();
                    last_sent = event.seq;
                    yield Ok(encode_event(&event));
                }
            }
        }
    }
}

/// Convenience duration for tests and documentation.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::events::{DaemonError, Heartbeat};

    fn heartbeat(sequence: u64) -> SseEvent {
        SseEvent::new(
            sequence,
            Utc::now(),
            SseEventKind::Heartbeat {
                data: Heartbeat {
                    uptime_secs: sequence,
                    generation: sequence,
                    node_count: 0,
                    edge_count: 0,
                    ts: Utc::now(),
                },
            },
        )
    }

    fn error_kind(message: &str) -> SseEventKind {
        SseEventKind::DaemonError {
            data: DaemonError {
                code: knect_proto::error::INTERNAL_ERROR,
                message: message.to_owned(),
                retryable: false,
                ts: Utc::now(),
            },
        }
    }

    #[test]
    fn frame_format_matches_the_contract() {
        let event = heartbeat(4);
        let frame = format_frame(&event);
        assert!(frame.starts_with(&format!(
            "id: {}\nevent: daemon.heartbeat\ndata: ",
            event.seq
        )));
        assert!(frame.ends_with("\n\n"));
        let lines: Vec<&str> = frame.trim_end_matches('\n').split('\n').collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].starts_with("data: {"));
        let payload = lines[2].trim_start_matches("data: ");
        let decoded: SseEvent = serde_json::from_str(payload).expect("frame data is JSON");
        assert_eq!(decoded.event_type(), "daemon.heartbeat");
        assert_eq!(decoded.seq, event.seq);
    }

    #[test]
    fn event_hub_assigns_monotonic_sequences_and_replays() {
        let hub = EventHub::new(10);
        let first = hub.publish(error_kind("one"));
        let second = hub.publish(error_kind("two"));
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(hub.len(), 2);

        assert_eq!(hub.replay_after(None), Replay::Snapshot);
        match hub.replay_after(Some(0)) {
            Replay::Events(events) => assert_eq!(events.len(), 2),
            other => panic!("unexpected replay: {other:?}"),
        }
        match hub.replay_after(Some(1)) {
            Replay::Events(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].seq, 2);
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn ring_overflow_requests_a_snapshot() {
        let hub = EventHub::new(2);
        hub.publish(error_kind("one"));
        hub.publish(error_kind("two"));
        hub.publish(error_kind("three"));
        assert_eq!(hub.len(), 2);
        assert_eq!(hub.replay_after(Some(0)), Replay::Snapshot);
        match hub.replay_after(Some(1)) {
            Replay::Events(events) => {
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].seq, 2);
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn heartbeat_interval_is_fifteen_seconds() {
        assert_eq!(HEARTBEAT_INTERVAL, Duration::from_secs(15));
    }
}
