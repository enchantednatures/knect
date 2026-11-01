//! Tap event retention and authenticated callback delivery.

use knect_proto::tap::TapEvent;

use super::entry::lock;
use super::manager::TriggerTapManager;

impl TriggerTapManager {
    /// Returns the events retained for a tap, active or recently retired.
    #[must_use]
    pub fn events(&self, tap_id: &str) -> Vec<TapEvent> {
        if let Some(entry) = lock(&self.inner.taps).get(tap_id).cloned() {
            return entry.events();
        }
        lock(&self.inner.retired)
            .iter()
            .find(|retired| retired.entry.id == tap_id)
            .map_or_else(Vec::new, |retired| retired.entry.events())
    }
}
