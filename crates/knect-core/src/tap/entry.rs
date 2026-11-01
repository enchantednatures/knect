//! Per-tap state and bounded event ring.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use knect_proto::tap::{TapEvent, TapFilter, TapHandle, TapState};

use super::trigger::TriggerCoordinates;

pub(super) struct TapEntry {
    pub(super) id: String,
    pub(super) token: String,
    pub(super) coordinates: TriggerCoordinates,
    pub(super) handle: Mutex<TapHandle>,
    pub(super) filter: Option<TapFilter>,
    pub(super) ring: Mutex<VecDeque<TapEvent>>,
    pub(super) max_events: usize,
    pub(super) expires: DateTime<Utc>,
}

impl TapEntry {
    pub(super) fn handle(&self) -> TapHandle {
        lock(&self.handle).clone()
    }

    pub(super) fn set_state(&self, state: TapState) {
        lock(&self.handle).state = state;
    }

    pub(super) fn is_active(&self) -> bool {
        lock(&self.handle).state == TapState::Active
    }

    pub(super) fn events(&self) -> Vec<TapEvent> {
        lock(&self.ring).iter().cloned().collect()
    }

    pub(super) fn push(&self, event: TapEvent) {
        let mut ring = lock(&self.ring);
        if ring.len() == self.max_events {
            ring.pop_front();
        }
        ring.push_back(event);
    }
}

pub(super) struct RetiredTap {
    pub(super) entry: Arc<TapEntry>,
    pub(super) retired_at: DateTime<Utc>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
