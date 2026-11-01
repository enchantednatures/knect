//! Real `TapManager`: ephemeral Trigger lifecycle, callback delivery, ring, TTL.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use futures::future::BoxFuture;
use knect_proto::rpc::{TapStartParams, TapStopResult};
use knect_proto::tap::{TapHandle, TapState};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::TapManager;
use super::callback::{TapCallbackRequest, TapDelivery, parse_cloud_event};
use super::config::TapManagerConfig;
use super::entry::{RetiredTap, TapEntry, lock};
use super::gc::gc_orphans;
use super::token::{generate_token, tokens_match};
use super::trigger::{self, CallbackRef, TriggerCoordinates, TriggerRequest};
use crate::error::CoreError;
use crate::kube::ClusterReader;

/// How long expired tap handles remain visible through `tap.list`.
pub(super) const RETIRED_RETENTION_SECS: i64 = 60;

pub(super) struct Inner {
    pub(super) taps: Mutex<BTreeMap<String, Arc<TapEntry>>>,
    pub(super) retired: Mutex<VecDeque<RetiredTap>>,
    pub(super) callback_addr: Mutex<Option<SocketAddr>>,
    pub(super) reaper_cancel: CancellationToken,
    pub(super) gc_done: AtomicBool,
    pub(super) reaper_started: AtomicBool,
}

/// Tap manager backed by ephemeral Knative Triggers.
#[derive(Clone)]
pub struct TriggerTapManager {
    pub(super) reader: Arc<dyn ClusterReader>,
    pub(super) config: TapManagerConfig,
    pub(super) inner: Arc<Inner>,
    pub(super) lifecycle: Arc<tokio::sync::Mutex<()>>,
}

impl TriggerTapManager {
    /// Creates a manager. Nothing contacts the cluster until `start` or GC.
    #[must_use]
    pub fn new(reader: Arc<dyn ClusterReader>, config: TapManagerConfig) -> Self {
        Self {
            reader,
            config,
            inner: Arc::new(Inner {
                taps: Mutex::new(BTreeMap::new()),
                retired: Mutex::new(VecDeque::new()),
                callback_addr: Mutex::new(None),
                reaper_cancel: CancellationToken::new(),
                gc_done: AtomicBool::new(false),
                reaper_started: AtomicBool::new(false),
            }),
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Deletes daemon-managed Triggers and Secrets left by an earlier daemon.
    pub async fn gc_startup(&self) -> Result<Vec<String>, CoreError> {
        let _guard = self.lifecycle.lock().await;
        let active = self.active_ids();
        gc_orphans(&self.reader, &active).await
    }

    fn active_ids(&self) -> BTreeSet<String> {
        lock(&self.inner.taps).keys().cloned().collect()
    }

    async fn start_locked(&self, params: TapStartParams) -> Result<TapHandle, CoreError> {
        if !self.config.enabled {
            return Err(CoreError::Tap(
                "tap is disabled by configuration".to_owned(),
            ));
        }
        trigger::validate_target(&params.target)?;

        let created = Utc::now();
        let ttl_secs = params.ttl_secs.unwrap_or(self.config.ttl_secs).max(1);
        let ttl = chrono::Duration::from_std(Duration::from_secs(ttl_secs))
            .map_err(|_| CoreError::Tap("tap ttl is too large".to_owned()))?;
        let expires = created
            .checked_add_signed(ttl)
            .ok_or_else(|| CoreError::Tap("tap ttl is too large".to_owned()))?;

        let id = format!("tap-{}", Uuid::new_v4().simple());
        let token = generate_token();
        let callback_url = trigger::callback_url(
            &self.config,
            self.callback_addr(),
            &CallbackRef {
                id: &id,
                token: &token,
            },
        )?;
        let max_events = self.event_capacity(params.max_events);
        let signature = TapHandle {
            id: id.clone(),
            target: params.target.clone(),
            state: TapState::Starting,
            created,
            expires,
            max_events,
            callback_url,
            filter: params.filter.clone(),
        };
        let entry = Arc::new(TapEntry {
            id: id.clone(),
            token,
            coordinates: TriggerCoordinates {
                namespace: params.target.namespace.clone(),
                trigger_name: id.clone(),
                secret_name: trigger::secret_name_for(&id),
            },
            handle: Mutex::new(signature),
            filter: params.filter.clone(),
            ring: Mutex::new(VecDeque::new()),
            max_events: usize::try_from(max_events).unwrap_or(usize::MAX),
            expires,
        });

        {
            let mut taps = lock(&self.inner.taps);
            let capacity = usize::try_from(self.config.max_taps.max(1)).unwrap_or(usize::MAX);
            if taps.len() >= capacity {
                return Err(CoreError::Tap(format!(
                    "tap limit reached: {} active taps",
                    taps.len()
                )));
            }
            taps.insert(id.clone(), Arc::clone(&entry));
        }

        let callback_url = entry.handle().callback_url;
        let request = TriggerRequest {
            id: &entry.id,
            target: &params.target,
            namespace: &entry.coordinates.namespace,
            trigger_name: &entry.coordinates.trigger_name,
            secret_name: &entry.coordinates.secret_name,
            token: &entry.token,
            callback_url: &callback_url,
            filter: params.filter.as_ref(),
        };
        if let Err(error) = trigger::create_trigger(&self.reader, request).await {
            lock(&self.inner.taps).remove(&id);
            return Err(error);
        }

        entry.set_state(TapState::Active);
        self.ensure_reaper();
        Ok(entry.handle())
    }

    fn event_capacity(&self, requested: Option<u32>) -> u32 {
        requested
            .unwrap_or(self.config.max_events)
            .min(self.config.max_events)
            .max(1)
    }

    fn callback_addr(&self) -> Option<SocketAddr> {
        *lock(&self.inner.callback_addr)
    }

    async fn ensure_startup_gc(&self) {
        if self.inner.gc_done.load(Ordering::Acquire) {
            return;
        }
        match gc_orphans(&self.reader, &self.active_ids()).await {
            Ok(removed) => {
                self.inner.gc_done.store(true, Ordering::Release);
                if !removed.is_empty() {
                    tracing::info!(removed = removed.len(), "reaped orphaned tap resources");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "tap startup GC failed; will retry on next start");
            }
        }
    }

    fn ensure_reaper(&self) {
        if self.config.ttl_secs == 0 || self.inner.reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = self.clone();
        let interval = (self.config.ttl_secs / 10).clamp(1, 30);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(interval));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = manager.inner.reaper_cancel.cancelled() => break,
                    _ = ticker.tick() => {
                        if let Err(error) = manager.reap().await {
                            tracing::warn!(%error, "tap reaper pass failed");
                        }
                    }
                }
            }
        });
    }
}

impl TapManager for TriggerTapManager {
    fn start(&self, params: TapStartParams) -> BoxFuture<'_, Result<TapHandle, CoreError>> {
        Box::pin(async move {
            let _guard = self.lifecycle.lock().await;
            if let Err(error) = self.reap_locked(Utc::now()).await {
                tracing::warn!(%error, "tap reap before start failed");
            }
            self.ensure_startup_gc().await;
            self.start_locked(params).await
        })
    }

    fn stop(&self, tap_id: String) -> BoxFuture<'_, Result<TapStopResult, CoreError>> {
        Box::pin(async move {
            let _guard = self.lifecycle.lock().await;
            let Some(entry) = lock(&self.inner.taps).remove(&tap_id) else {
                return Ok(TapStopResult { removed: false });
            };
            entry.set_state(TapState::Stopped);
            trigger::delete_trigger_resources(&self.reader, &entry.coordinates).await?;
            Ok(TapStopResult { removed: true })
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<TapHandle>, CoreError>> {
        Box::pin(async move {
            let mut handles: Vec<TapHandle> = lock(&self.inner.taps)
                .values()
                .map(|entry| entry.handle())
                .collect();
            let cutoff = Utc::now() - chrono::Duration::seconds(RETIRED_RETENTION_SECS);
            let mut retired = lock(&self.inner.retired);
            while retired
                .front()
                .is_some_and(|retired| retired.retired_at < cutoff)
            {
                retired.pop_front();
            }
            handles.extend(retired.iter().map(|retired| retired.entry.handle()));
            handles.sort_by(|left, right| left.id.cmp(&right.id));
            Ok(handles)
        })
    }

    fn set_callback_addr(&self, addr: SocketAddr) {
        *lock(&self.inner.callback_addr) = Some(addr);
    }

    fn active_count(&self) -> usize {
        lock(&self.inner.taps).len()
    }

    fn shutdown(&self) -> BoxFuture<'_, Result<(), CoreError>> {
        Box::pin(async move {
            let _guard = self.lifecycle.lock().await;
            self.inner.reaper_cancel.cancel();
            let taps: Vec<Arc<TapEntry>> = {
                let mut taps = lock(&self.inner.taps);
                std::mem::take(&mut *taps).into_values().collect()
            };
            for entry in &taps {
                entry.set_state(TapState::Stopped);
                if let Err(error) =
                    trigger::delete_trigger_resources(&self.reader, &entry.coordinates).await
                {
                    tracing::warn!(
                        tap_id = %entry.id,
                        %error,
                        "tap shutdown cleanup failed; startup GC will retry"
                    );
                }
            }
            lock(&self.inner.retired).clear();
            Ok(())
        })
    }

    fn deliver(
        &self,
        tap_id: &str,
        token: &str,
        request: TapCallbackRequest,
    ) -> BoxFuture<'_, Result<TapDelivery, CoreError>> {
        let tap_id = tap_id.to_owned();
        let token = token.to_owned();
        Box::pin(async move {
            let entry = lock(&self.inner.taps)
                .get(&tap_id)
                .cloned()
                .ok_or_else(|| CoreError::Tap("tap callback tap not found".to_owned()))?;
            if entry.expires <= Utc::now() {
                return Err(CoreError::Tap("tap callback tap expired".to_owned()));
            }
            if !entry.is_active() {
                return Err(CoreError::Tap("tap callback tap is not active".to_owned()));
            }
            if !tokens_match(&entry.token, &token) {
                return Err(CoreError::Tap("unauthorized tap callback".to_owned()));
            }
            match parse_cloud_event(request, entry.filter.as_ref())? {
                Some(event) => {
                    entry.push(event.clone());
                    Ok(TapDelivery::Published(event))
                }
                None => Ok(TapDelivery::Filtered),
            }
        })
    }

    fn reap(&self) -> BoxFuture<'_, Result<Vec<String>, CoreError>> {
        Box::pin(async move {
            let _guard = self.lifecycle.lock().await;
            self.reap_locked(Utc::now()).await
        })
    }
}
