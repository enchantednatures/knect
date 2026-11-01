//! Settings for the Trigger-backed tap manager.

use crate::config::TapConfig;

/// Settings for [`super::TriggerTapManager`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapManagerConfig {
    /// Whether starting taps is allowed.
    pub enabled: bool,
    /// Callback listener bind address.
    pub callback_bind: String,
    /// Host advertised to Knative. Empty auto-detects the bind address.
    pub callback_host: String,
    /// Maximum concurrent taps.
    pub max_taps: u32,
    /// Maximum retained events per tap.
    pub max_events: u32,
    /// Default tap time-to-live in seconds.
    pub ttl_secs: u64,
}

impl Default for TapManagerConfig {
    fn default() -> Self {
        let config = TapConfig::default();
        Self::from(&config)
    }
}

impl From<&TapConfig> for TapManagerConfig {
    fn from(config: &TapConfig) -> Self {
        Self {
            enabled: config.enabled,
            callback_bind: config.callback_bind.clone(),
            callback_host: config.callback_host.clone(),
            max_taps: config.max_taps,
            max_events: config.max_events,
            ttl_secs: config.ttl_secs,
        }
    }
}

impl From<TapConfig> for TapManagerConfig {
    fn from(config: TapConfig) -> Self {
        Self::from(&config)
    }
}
