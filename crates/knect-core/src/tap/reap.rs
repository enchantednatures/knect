//! TTL expiry and retired-handle retention.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use knect_proto::tap::TapState;

use super::entry::{RetiredTap, TapEntry, lock};
use super::manager::{RETIRED_RETENTION_SECS, TriggerTapManager};
use super::trigger;
use crate::error::CoreError;

impl TriggerTapManager {
    /// Reaps taps whose expiry is at or before `now`.
    pub async fn reap_at(&self, now: DateTime<Utc>) -> Result<Vec<String>, CoreError> {
        let _guard = self.lifecycle.lock().await;
        self.reap_locked(now).await
    }

    pub(super) async fn reap_locked(&self, now: DateTime<Utc>) -> Result<Vec<String>, CoreError> {
        let expired: Vec<Arc<TapEntry>> = {
            let mut taps = lock(&self.inner.taps);
            let ids: Vec<String> = taps
                .iter()
                .filter(|(_, entry)| entry.expires <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter().filter_map(|id| taps.remove(&id)).collect()
        };
        let ids: Vec<String> = expired.iter().map(|entry| entry.id.clone()).collect();
        for entry in &expired {
            entry.set_state(TapState::Expired);
            if let Err(error) =
                trigger::delete_trigger_resources(&self.reader, &entry.coordinates).await
            {
                tracing::warn!(
                    tap_id = %entry.id,
                    %error,
                    "expired tap resource cleanup failed; startup GC will retry"
                );
            }
        }
        if !expired.is_empty() {
            let mut retired = lock(&self.inner.retired);
            for entry in expired {
                retired.push_back(RetiredTap {
                    entry,
                    retired_at: now,
                });
            }
            let cutoff = now - chrono::Duration::seconds(RETIRED_RETENTION_SECS);
            while retired
                .front()
                .is_some_and(|retired| retired.retired_at < cutoff)
            {
                retired.pop_front();
            }
        }
        Ok(ids)
    }
}
