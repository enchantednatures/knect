//! Configuration loading and precedence.
//!
//! Precedence, highest first: explicit [`ConfigOverrides`] (CLI flags) ->
//! `KNECT_*` environment variables -> TOML file -> serde defaults. Unknown TOML
//! fields are rejected so a typo cannot silently disable a guard. The daemon
//! exposes [`Config::safe`] only; no raw token or kubeconfig content lives in
//! this type.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use knect_proto::rpc::{
    SafeActionsConfig, SafeCacheConfig, SafeConfig, SafeDaemonConfig, SafeDiscoveryConfig,
    SafeTapConfig, SafeUiConfig, SafeWatchConfig,
};

/// Default API groups watched by the daemon.
///
/// The empty string is the core API group. Core and `apps` resources are
/// required to complete the Serving owner chain (Pod/Service/Endpoints) and
/// the storage and policy correlators (PVC/PV/Pod).
pub const DEFAULT_DISCOVERY_GROUPS: &[&str] = &[
    "",
    "apps",
    "autoscaling.internal.knative.dev",
    "serving.knative.dev",
    "eventing.knative.dev",
    "sources.knative.dev",
    "messaging.knative.dev",
    "networking.istio.io",
    "security.istio.io",
    "kyverno.io",
    "wgpolicyk8s.io",
    "flagger.app",
    "kafka.strimzi.io",
    "cluster.redpanda.com",
    "rabbitmq.com",
    "dragonflydb.io",
    "redis.redis.opstreelabs.in",
    "postgresql.cnpg.io",
    "storage.k8s.io",
    "ceph.rook.io",
];

/// Default annotations stripped before objects enter the store.
pub const DEFAULT_STRIP_ANNOTATIONS: &[&str] = &[
    "kubectl.kubernetes.io/last-applied-configuration",
    "deployment.kubernetes.io/revision",
    "control-plane.alpha.kubernetes.io/leader",
];

/// Complete daemon configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Daemon transport, logging, and safety settings.
    pub daemon: DaemonConfig,
    /// API discovery settings.
    pub discovery: DiscoveryConfig,
    /// Watch and sanitization settings.
    pub watch: WatchConfig,
    /// Snapshot cache settings.
    pub cache: CacheConfig,
    /// CloudEvent tap settings.
    pub tap: TapConfig,
    /// Presentation settings shared with the TUI and WebUI.
    pub ui: UiConfig,
    /// Guarded write action settings.
    pub actions: ActionsConfig,
}

/// Daemon transport and logging settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    /// Unix domain socket path.
    pub socket_path: String,
    /// HTTP listen address.
    pub http_addr: String,
    /// Cache directory. `~` is expanded.
    pub cache_dir: String,
    /// Kubernetes context name. Empty means the kubeconfig current context.
    pub context: String,
    /// Tracing filter directive.
    pub log: String,
    /// Log format: `human` or `json`.
    pub log_format: String,
    /// Master switch for guarded write actions.
    pub allow_actions: bool,
    /// Allow non-loopback HTTP binds.
    pub allow_remote: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: "/tmp/knectd.sock".to_owned(),
            http_addr: "127.0.0.1:7420".to_owned(),
            cache_dir: "~/.cache/knect".to_owned(),
            context: String::new(),
            log: "info".to_owned(),
            log_format: "human".to_owned(),
            allow_actions: false,
            allow_remote: false,
        }
    }
}

/// API discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscoveryConfig {
    /// Rediscovery interval in seconds.
    pub rediscover_secs: u64,
    /// API groups eligible for discovery.
    pub groups: Vec<String>,
    /// Namespaces watched. Empty means all namespaces.
    pub namespaces: Vec<String>,
    /// Watch core `Secret` objects. Disabled by default so the observer never
    /// reads Secret material unless the operator opts in.
    pub secrets: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            rediscover_secs: 60,
            groups: DEFAULT_DISCOVERY_GROUPS
                .iter()
                .map(|group| (*group).to_owned())
                .collect(),
            namespaces: Vec::new(),
            secrets: false,
        }
    }
}

/// Watch and sanitization settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WatchConfig {
    /// Recompute debounce in milliseconds.
    pub debounce_ms: u64,
    /// Maximum number of tracked objects.
    pub max_objects: u64,
    /// Annotation keys stripped before objects enter the store.
    pub strip_annotations: Vec<String>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 150,
            max_objects: 20_000,
            strip_annotations: DEFAULT_STRIP_ANNOTATIONS
                .iter()
                .map(|annotation| (*annotation).to_owned())
                .collect(),
        }
    }
}

/// Snapshot cache settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    /// Whether the cache is enabled.
    pub enabled: bool,
    /// Flush interval in milliseconds.
    pub flush_ms: u64,
    /// Maximum serialized snapshot size in bytes.
    pub max_snapshot_bytes: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            flush_ms: 500,
            max_snapshot_bytes: 33_554_432,
        }
    }
}

/// CloudEvent tap settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TapConfig {
    /// Whether taps are enabled.
    pub enabled: bool,
    /// Callback listener bind address.
    pub callback_bind: String,
    /// Callback host advertised to Knative. Empty means auto-detect.
    pub callback_host: String,
    /// Maximum concurrent taps.
    pub max_taps: u32,
    /// Maximum events retained per tap.
    pub max_events: u32,
    /// Tap time-to-live in seconds.
    pub ttl_secs: u64,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            callback_bind: "0.0.0.0:7421".to_owned(),
            callback_host: String::new(),
            max_taps: 5,
            max_events: 1_000,
            ttl_secs: 300,
        }
    }
}

/// Presentation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Target frames per second.
    pub fps: u32,
    /// Theme name.
    pub theme: String,
    /// Whether to use ASCII-only rendering.
    pub ascii: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            fps: 60,
            theme: "knect-dark".to_owned(),
            ascii: false,
        }
    }
}

/// Guarded write action settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActionsConfig {
    /// Whether actions are enabled.
    pub enabled: bool,
}

/// Explicit CLI overrides applied after file and environment values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigOverrides {
    /// `--context`
    pub context: Option<String>,
    /// `--kubeconfig` (not part of [`Config`]; consumed by the reader factory)
    pub kubeconfig: Option<PathBuf>,
    /// `--http-addr`
    pub http_addr: Option<String>,
    /// `--socket-path`
    pub socket_path: Option<PathBuf>,
    /// `--cache-dir`
    pub cache_dir: Option<PathBuf>,
    /// `--allow-actions`
    pub allow_actions: Option<bool>,
    /// `--log`
    pub log: Option<String>,
    /// `--log-format`
    pub log_format: Option<String>,
}

/// Source of environment variables, injectable for deterministic tests.
pub trait EnvSource {
    /// Returns the value for `key`, if set.
    fn get(&self, key: &str) -> Option<String>;
}

/// Reads the real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl EnvSource for BTreeMap<String, String> {
    fn get(&self, key: &str) -> Option<String> {
        self.get(key).cloned()
    }
}

impl Config {
    /// Loads configuration using the real process environment.
    ///
    /// A `None` path uses the XDG default (`~/.config/knect/config.toml`) when
    /// it exists. A missing default file is not an error.
    pub fn load(path: Option<&Path>, overrides: ConfigOverrides) -> Result<Self, CoreError> {
        Self::load_with_env(path, overrides, &ProcessEnv)
    }

    /// Loads configuration with an injected environment.
    ///
    /// This exists so tests can prove precedence without mutating the process
    /// environment (which is unsafe in edition 2024).
    pub fn load_with_env<E: EnvSource + ?Sized>(
        path: Option<&Path>,
        overrides: ConfigOverrides,
        env: &E,
    ) -> Result<Self, CoreError> {
        let mut config = match path {
            Some(path) => Self::from_file(path)?,
            None => match Self::default_path() {
                Some(path) if path.is_file() => Self::from_file(&path)?,
                _ => Self::default(),
            },
        };
        config.apply_env(env)?;
        config.apply_overrides(overrides)?;
        config.normalize();
        config.validate()?;
        Ok(config)
    }

    /// Returns the XDG configuration path when the home directory is known.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|base| base.config_dir().join("knect").join("config.toml"))
    }

    /// Parses a TOML file, rejecting unknown fields.
    pub fn from_file(path: &Path) -> Result<Self, CoreError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            CoreError::Config(format!("cannot read {}: {error}", path.display()))
        })?;
        toml::from_str(&raw)
            .map_err(|error| CoreError::Config(format!("invalid {}: {error}", path.display())))
    }

    /// Applies every supported `KNECT_*` environment variable.
    pub fn apply_env<E: EnvSource + ?Sized>(&mut self, env: &E) -> Result<(), CoreError> {
        if let Some(value) = env.get("KNECT_DAEMON_HTTP_ADDR") {
            self.daemon.http_addr = value;
        }
        if let Some(value) = env.get("KNECT_DAEMON_SOCKET_PATH") {
            self.daemon.socket_path = value;
        }
        if let Some(value) = env
            .get("KNECT_DAEMON_CACHE_DIR")
            .or_else(|| env.get("KNECT_CACHE_DIR"))
        {
            self.daemon.cache_dir = value;
        }
        if let Some(value) = env.get("KNECT_DAEMON_CONTEXT") {
            self.daemon.context = value;
        }
        if let Some(value) = env.get("KNECT_DAEMON_LOG").or_else(|| env.get("KNECT_LOG")) {
            self.daemon.log = value;
        }
        if let Some(value) = env
            .get("KNECT_DAEMON_LOG_FORMAT")
            .or_else(|| env.get("KNECT_LOG_FORMAT"))
        {
            self.daemon.log_format = value;
        }
        if let Some(value) = env.get("KNECT_DAEMON_ALLOW_ACTIONS") {
            self.daemon.allow_actions = parse_bool("KNECT_DAEMON_ALLOW_ACTIONS", &value)?;
        }
        if let Some(value) = env.get("KNECT_ALLOW_REMOTE") {
            self.daemon.allow_remote = parse_bool("KNECT_ALLOW_REMOTE", &value)?;
        }
        if let Some(value) = env.get("KNECT_DISCOVERY_REDISCOVER_SECS") {
            self.discovery.rediscover_secs = parse_u64("KNECT_DISCOVERY_REDISCOVER_SECS", &value)?;
        }
        if let Some(value) = env.get("KNECT_DISCOVERY_SECRETS") {
            self.discovery.secrets = parse_bool("KNECT_DISCOVERY_SECRETS", &value)?;
        }
        if let Some(value) = env.get("KNECT_WATCH_DEBOUNCE_MS") {
            self.watch.debounce_ms = parse_u64("KNECT_WATCH_DEBOUNCE_MS", &value)?;
        }
        if let Some(value) = env.get("KNECT_WATCH_MAX_OBJECTS") {
            self.watch.max_objects = parse_u64("KNECT_WATCH_MAX_OBJECTS", &value)?;
        }
        if let Some(value) = env.get("KNECT_CACHE_ENABLED") {
            self.cache.enabled = parse_bool("KNECT_CACHE_ENABLED", &value)?;
        }
        if let Some(value) = env.get("KNECT_CACHE_FLUSH_MS") {
            self.cache.flush_ms = parse_u64("KNECT_CACHE_FLUSH_MS", &value)?;
        }
        if let Some(value) = env.get("KNECT_TAP_ENABLED") {
            self.tap.enabled = parse_bool("KNECT_TAP_ENABLED", &value)?;
        }
        if let Some(value) = env.get("KNECT_TAP_CALLBACK_BIND") {
            self.tap.callback_bind = value;
        }
        if let Some(value) = env.get("KNECT_TAP_CALLBACK_HOST") {
            self.tap.callback_host = value;
        }
        if let Some(value) = env.get("KNECT_UI_FPS") {
            self.ui.fps = parse_u32("KNECT_UI_FPS", &value)?;
        }
        if let Some(value) = env.get("KNECT_ACTIONS_ENABLED") {
            self.actions.enabled = parse_bool("KNECT_ACTIONS_ENABLED", &value)?;
        }
        Ok(())
    }

    /// Applies explicit CLI overrides last.
    pub fn apply_overrides(&mut self, overrides: ConfigOverrides) -> Result<(), CoreError> {
        if let Some(context) = overrides.context {
            self.daemon.context = context;
        }
        if let Some(http_addr) = overrides.http_addr {
            self.daemon.http_addr = http_addr;
        }
        if let Some(socket_path) = overrides.socket_path {
            self.daemon.socket_path = socket_path.to_string_lossy().into_owned();
        }
        if let Some(cache_dir) = overrides.cache_dir {
            self.daemon.cache_dir = cache_dir.to_string_lossy().into_owned();
        }
        if let Some(allow_actions) = overrides.allow_actions {
            self.daemon.allow_actions = allow_actions;
            self.actions.enabled = allow_actions;
        }
        if let Some(log) = overrides.log {
            self.daemon.log = log;
        }
        if let Some(log_format) = overrides.log_format {
            self.daemon.log_format = log_format;
        }
        Ok(())
    }

    /// Expands `~` in filesystem paths.
    pub fn normalize(&mut self) {
        self.daemon.socket_path =
            expand_tilde_with_home(&self.daemon.socket_path, home_dir().as_deref());
        self.daemon.cache_dir =
            expand_tilde_with_home(&self.daemon.cache_dir, home_dir().as_deref());
    }

    /// Validates cross-field invariants that serde cannot express.
    pub fn validate(&self) -> Result<(), CoreError> {
        match self.daemon.log_format.as_str() {
            "human" | "json" => {}
            other => {
                return Err(CoreError::Config(format!(
                    "daemon.log_format must be 'human' or 'json', got '{other}'"
                )));
            }
        }
        if self.daemon.log.trim().is_empty() {
            return Err(CoreError::Config("daemon.log must not be empty".to_owned()));
        }
        Ok(())
    }

    /// Projects the safe configuration returned by `config.get`.
    ///
    /// There is no field for tokens, kubeconfig content, or secret values.
    #[must_use]
    pub fn safe(&self) -> SafeConfig {
        SafeConfig {
            daemon: SafeDaemonConfig {
                socket_path: self.daemon.socket_path.clone(),
                http_addr: self.daemon.http_addr.clone(),
                cache_dir: self.daemon.cache_dir.clone(),
                context: self.daemon.context.clone(),
                log: self.daemon.log.clone(),
                log_format: self.daemon.log_format.clone(),
                allow_actions: self.daemon.allow_actions,
                allow_remote: self.daemon.allow_remote,
            },
            discovery: SafeDiscoveryConfig {
                rediscover_secs: self.discovery.rediscover_secs,
                groups: self.discovery.groups.clone(),
                namespaces: self.discovery.namespaces.clone(),
            },
            watch: SafeWatchConfig {
                debounce_ms: self.watch.debounce_ms,
                max_objects: self.watch.max_objects,
                strip_annotations: self.watch.strip_annotations.clone(),
            },
            cache: SafeCacheConfig {
                enabled: self.cache.enabled,
                flush_ms: self.cache.flush_ms,
                max_snapshot_bytes: self.cache.max_snapshot_bytes,
            },
            tap: SafeTapConfig {
                enabled: self.tap.enabled,
                callback_bind: self.tap.callback_bind.clone(),
                callback_host: self.tap.callback_host.clone(),
                max_taps: self.tap.max_taps,
                max_events: self.tap.max_events,
                ttl_secs: self.tap.ttl_secs,
            },
            ui: SafeUiConfig {
                fps: self.ui.fps,
                theme: self.ui.theme.clone(),
                ascii: self.ui.ascii,
            },
            actions: SafeActionsConfig {
                enabled: self.actions.enabled,
            },
        }
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, CoreError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(CoreError::Config(format!(
            "{key} must be a boolean, got '{value}'"
        ))),
    }
}

fn parse_u64(key: &str, value: &str) -> Result<u64, CoreError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|error| CoreError::Config(format!("{key} must be an unsigned integer: {error}")))
}

fn parse_u32(key: &str, value: &str) -> Result<u32, CoreError> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|error| CoreError::Config(format!("{key} must be an unsigned integer: {error}")))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|base| base.home_dir().to_path_buf()))
}

fn expand_tilde_with_home(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    if path == "~" {
        return home.to_string_lossy().into_owned();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).expect("write config");
        (dir, path)
    }

    fn empty_env() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn defaults_match_the_plan() {
        // Use an explicit empty file so a developer's real
        // ~/.config/knect/config.toml cannot change the assertion.
        let (_dir, path) = file("");
        let config = Config::load_with_env(Some(&path), ConfigOverrides::default(), &empty_env())
            .expect("defaults load");
        assert_eq!(config.daemon.socket_path, "/tmp/knectd.sock");
        assert_eq!(config.daemon.http_addr, "127.0.0.1:7420");
        assert!(config.daemon.cache_dir.ends_with(".cache/knect"));
        assert_eq!(config.daemon.context, "");
        assert_eq!(config.daemon.log, "info");
        assert_eq!(config.daemon.log_format, "human");
        assert!(!config.daemon.allow_actions);
        assert!(!config.daemon.allow_remote);
        assert_eq!(config.discovery.rediscover_secs, 60);
        assert_eq!(
            config.discovery.groups.len(),
            DEFAULT_DISCOVERY_GROUPS.len()
        );
        assert!(config.discovery.groups.iter().any(String::is_empty));
        assert!(config.discovery.groups.iter().any(|group| group == "apps"));
        assert!(!config.discovery.secrets);
        assert_eq!(config.watch.debounce_ms, 150);
        assert_eq!(config.watch.max_objects, 20_000);
        assert!(config.cache.enabled);
        assert_eq!(config.cache.flush_ms, 500);
        assert_eq!(config.cache.max_snapshot_bytes, 33_554_432);
        assert!(config.tap.enabled);
        assert_eq!(config.tap.callback_bind, "0.0.0.0:7421");
        assert_eq!(config.tap.max_taps, 5);
        assert_eq!(config.tap.max_events, 1_000);
        assert_eq!(config.tap.ttl_secs, 300);
        assert_eq!(config.ui.fps, 60);
        assert_eq!(config.ui.theme, "knect-dark");
        assert!(!config.ui.ascii);
        assert!(!config.actions.enabled);
    }

    #[test]
    fn file_values_are_loaded_and_expanded() {
        let (_dir, path) = file(
            r#"
[daemon]
http_addr = "127.0.0.1:9000"
cache_dir = "~/custom-cache"
context = "from-file"
allow_actions = true

[ui]
fps = 30
"#,
        );
        let config = Config::load_with_env(Some(&path), ConfigOverrides::default(), &empty_env())
            .expect("file loads");
        assert_eq!(config.daemon.http_addr, "127.0.0.1:9000");
        assert_eq!(config.daemon.context, "from-file");
        assert!(config.daemon.allow_actions);
        assert_eq!(config.ui.fps, 30);
        assert!(!config.daemon.cache_dir.contains('~'));
        assert!(config.daemon.cache_dir.ends_with("custom-cache"));
    }

    #[test]
    fn env_overrides_file_and_overrides_override_env() {
        let (_dir, path) = file(
            r#"
[daemon]
http_addr = "127.0.0.1:9000"
context = "from-file"
log = "warn"
"#,
        );
        let mut env = BTreeMap::new();
        env.insert(
            "KNECT_DAEMON_HTTP_ADDR".to_owned(),
            "127.0.0.1:9100".to_owned(),
        );
        env.insert("KNECT_DAEMON_CONTEXT".to_owned(), "from-env".to_owned());
        env.insert("KNECT_LOG".to_owned(), "error".to_owned());
        env.insert("KNECT_TAP_CALLBACK_HOST".to_owned(), "env-host".to_owned());
        env.insert("KNECT_DISCOVERY_SECRETS".to_owned(), "true".to_owned());

        let overrides = ConfigOverrides {
            context: Some("from-cli".to_owned()),
            socket_path: Some(PathBuf::from("/tmp/cli.sock")),
            ..ConfigOverrides::default()
        };
        let config = Config::load_with_env(Some(&path), overrides, &env).expect("loads");
        assert_eq!(config.daemon.http_addr, "127.0.0.1:9100");
        assert_eq!(config.daemon.context, "from-cli");
        assert_eq!(config.daemon.log, "error");
        assert_eq!(config.daemon.socket_path, "/tmp/cli.sock");
        assert_eq!(config.tap.callback_host, "env-host");
        assert!(config.discovery.secrets);
    }

    #[test]
    fn allow_actions_override_enables_both_gates() {
        let mut config = Config::default();
        config
            .apply_overrides(ConfigOverrides {
                allow_actions: Some(true),
                ..ConfigOverrides::default()
            })
            .expect("override applies");
        assert!(config.daemon.allow_actions);
        assert!(config.actions.enabled);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let (_dir, path) = file(
            r#"
[daemon]
http_addr = "127.0.0.1:9000"
bogus = true
"#,
        );
        let error = Config::load_with_env(Some(&path), ConfigOverrides::default(), &empty_env())
            .expect_err("unknown field must be rejected");
        assert!(error.to_string().contains("bogus"));
    }

    #[test]
    fn unknown_root_fields_are_rejected() {
        let (_dir, path) = file("mystery = 1\n");
        let error = Config::load_with_env(Some(&path), ConfigOverrides::default(), &empty_env())
            .expect_err("unknown root field must be rejected");
        assert!(error.to_string().contains("mystery"));
    }

    #[test]
    fn invalid_env_boolean_is_typed() {
        let mut env = BTreeMap::new();
        env.insert("KNECT_DAEMON_ALLOW_ACTIONS".to_owned(), "maybe".to_owned());
        let error = Config::load_with_env(None, ConfigOverrides::default(), &env)
            .expect_err("invalid boolean must fail");
        assert!(error.to_string().contains("KNECT_DAEMON_ALLOW_ACTIONS"));
    }

    #[test]
    fn tilde_expansion_uses_the_supplied_home() {
        assert_eq!(
            expand_tilde_with_home("~/.cache/knect", Some(Path::new("/home/test"))),
            "/home/test/.cache/knect"
        );
        assert_eq!(
            expand_tilde_with_home("~", Some(Path::new("/home/test"))),
            "/home/test"
        );
        assert_eq!(
            expand_tilde_with_home("/tmp/knect", Some(Path::new("/home/test"))),
            "/tmp/knect"
        );
        assert_eq!(expand_tilde_with_home("~/x", None), "~/x");
    }

    #[test]
    fn safe_config_never_contains_reader_inputs() {
        let (_dir, path) = file("");
        let config = Config::load_with_env(
            Some(&path),
            ConfigOverrides {
                context: Some("safe-context".to_owned()),
                kubeconfig: Some(PathBuf::from("/home/test/.kube/config")),
                ..ConfigOverrides::default()
            },
            &empty_env(),
        )
        .expect("loads");
        let safe = config.safe();
        assert_eq!(safe.daemon.context, "safe-context");
        assert_eq!(safe.daemon.http_addr, "127.0.0.1:7420");
        assert!(!safe.daemon.allow_actions);
        let json = serde_json::to_string(&safe).expect("safe config serializes");
        assert!(!json.contains("kubeconfig"));
        assert!(!json.contains("token"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn invalid_log_format_is_rejected() {
        let mut env = BTreeMap::new();
        env.insert("KNECT_LOG_FORMAT".to_owned(), "xml".to_owned());
        let error = Config::load_with_env(None, ConfigOverrides::default(), &env)
            .expect_err("invalid log format must fail");
        assert!(error.to_string().contains("log_format"));
    }
}
