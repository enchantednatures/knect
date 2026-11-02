//! Locating and launching `knectd` when no daemon is reachable.
//!
//! Auto-spawn is only attempted for the implicit default endpoint. A daemon
//! the user already had running is never killed: the handle is only created
//! when this process launched the child itself.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::net::UnixStream;
use tokio::process::{Child, Command};

use super::ClientError;

/// Name of the daemon executable.
pub const DAEMON_BINARY: &str = if cfg!(windows) {
    "knectd.exe"
} else {
    "knectd"
};

/// Locates the daemon binary.
///
/// Precedence: an explicit path, `KNECTD_BIN`, a sibling of the current
/// executable, then `PATH`.
#[must_use]
pub fn locate_daemon(explicit: Option<&Path>) -> Option<PathBuf> {
    let env_bin = std::env::var("KNECTD_BIN").ok();
    let current = std::env::current_exe().ok();
    let path = std::env::var_os("PATH");
    locate_daemon_with(
        explicit,
        env_bin.as_deref(),
        current.as_deref(),
        path.as_deref(),
    )
}

/// Pure form of [`locate_daemon`] used by tests and callers that already have
/// the environment inputs.
#[must_use]
pub fn locate_daemon_with(
    explicit: Option<&Path>,
    env_bin: Option<&str>,
    current_exe: Option<&Path>,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return is_executable(path).then(|| path.to_path_buf());
    }
    if let Some(value) = env_bin
        && !value.trim().is_empty()
    {
        let candidate = PathBuf::from(value);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        if !candidate.is_absolute()
            && let Some(found) = find_on_path(&candidate, path_env)
        {
            return Some(found);
        }
    }
    if let Some(current) = current_exe
        && let Some(directory) = current.parent()
    {
        let sibling = directory.join(DAEMON_BINARY);
        if is_executable(&sibling) {
            return Some(sibling);
        }
    }
    find_on_path(Path::new(DAEMON_BINARY), path_env)
}

fn find_on_path(name: &Path, path_env: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if name.components().count() > 1 {
        return None;
    }
    let path = path_env?;
    std::env::split_paths(path)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A daemon process launched by this client.
#[derive(Debug)]
pub struct SpawnedDaemon {
    child: Child,
    program: PathBuf,
    log_path: Option<PathBuf>,
}

impl SpawnedDaemon {
    /// Process ID of the spawned daemon, when still available.
    #[must_use]
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Path of the spawned program.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Captured daemon log, when the log file could be created.
    #[must_use]
    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    /// Sends a kill signal, ignoring errors on an already-exited child.
    pub fn terminate(&mut self) {
        let _ = self.child.start_kill();
    }

    /// Waits for the child to exit, bounded by `timeout`.
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        matches!(
            tokio::time::timeout(timeout, self.child.wait()).await,
            Ok(Ok(_))
        )
    }
}

impl Drop for SpawnedDaemon {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Spawns `knectd` bound to `socket` with inherited configuration.
///
/// The socket path is always overridden. Unless `KNECT_DAEMON_HTTP_ADDR` is
/// configured, the daemon binds an ephemeral loopback HTTP port so auto-spawn
/// cannot collide with a local service. Standard streams are detached from the
/// terminal; stderr is captured to a temporary log for diagnostics.
pub async fn spawn_daemon(program: &Path, socket: &Path) -> Result<SpawnedDaemon, ClientError> {
    let configured_http_addr = std::env::var("KNECT_DAEMON_HTTP_ADDR").ok();
    let log_path = std::env::temp_dir().join(format!("knectd-spawn-{}.log", std::process::id()));
    let (log_path, stderr) = match std::fs::File::create(&log_path) {
        Ok(file) => (Some(log_path), Stdio::from(file)),
        Err(_) => (None, Stdio::null()),
    };
    let mut command = Command::new(program);
    command
        .arg("--socket-path")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .kill_on_drop(true);
    if let Some(address) = auto_http_addr(configured_http_addr.as_deref()) {
        command.args(["--http-addr", address]);
    }
    let child = command.spawn().map_err(|error| {
        ClientError::Spawn(format!("cannot run {}: {error}", program.display()))
    })?;
    Ok(SpawnedDaemon {
        child,
        program: program.to_path_buf(),
        log_path,
    })
}

fn auto_http_addr(configured: Option<&str>) -> Option<&'static str> {
    configured
        .filter(|value| !value.trim().is_empty())
        .is_none()
        .then_some("127.0.0.1:0")
}

/// Waits until the Unix socket accepts a connection.
///
/// Polls every 50 ms so daemon start-up latency is visible quickly without a
/// busy loop.
pub async fn wait_for_socket(path: &Path, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if UnixStream::connect(path).await.is_ok() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_script(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, body).expect("write script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod");
        path
    }

    #[test]
    fn explicit_path_wins_and_must_be_executable() {
        let directory = tempfile::tempdir().expect("temp");
        let script = write_script(directory.path(), "my-knectd", "#!/bin/sh\nexit 0\n");
        assert_eq!(locate_daemon(Some(&script)), Some(script.clone()));
        let missing = directory.path().join("missing");
        assert_eq!(locate_daemon(Some(&missing)), None);
    }

    #[test]
    fn knectd_bin_env_is_consulted_before_path() {
        let directory = tempfile::tempdir().expect("temp");
        let script = write_script(directory.path(), "env-knectd", "#!/bin/sh\nexit 0\n");
        let empty_path = std::ffi::OsString::new();
        let found = locate_daemon_with(
            None,
            Some(script.to_str().expect("utf-8 path")),
            None,
            Some(&empty_path),
        );
        assert_eq!(found, Some(script));
    }

    #[test]
    fn sibling_binary_is_found_next_to_the_current_executable() {
        let directory = tempfile::tempdir().expect("temp");
        let script = write_script(directory.path(), DAEMON_BINARY, "#!/bin/sh\nexit 0\n");
        let fake_current = directory.path().join("knect");
        std::fs::write(&fake_current, b"binary").expect("write");
        let empty_path = std::ffi::OsString::new();
        assert_eq!(
            locate_daemon_with(None, None, Some(&fake_current), Some(&empty_path)),
            Some(script)
        );
    }

    #[test]
    fn path_lookup_finds_the_daemon() {
        let directory = tempfile::tempdir().expect("temp");
        let script = write_script(directory.path(), DAEMON_BINARY, "#!/bin/sh\nexit 0\n");
        let path = std::ffi::OsString::from(directory.path());
        assert_eq!(
            locate_daemon_with(None, None, None, Some(&path)),
            Some(script)
        );
    }

    #[tokio::test]
    async fn wait_for_socket_detects_a_bound_socket() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("knectd.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind");
        assert!(wait_for_socket(&path, Duration::from_millis(200)).await);
        drop(listener);
        std::fs::remove_file(&path).expect("remove socket");
        assert!(!wait_for_socket(&path, Duration::from_millis(100)).await);
    }

    #[test]
    fn auto_http_addr_is_used_only_when_unconfigured() {
        assert_eq!(auto_http_addr(None), Some("127.0.0.1:0"));
        assert_eq!(auto_http_addr(Some("")), Some("127.0.0.1:0"));
        assert_eq!(auto_http_addr(Some("  ")), Some("127.0.0.1:0"));
        assert_eq!(auto_http_addr(Some("127.0.0.1:9000")), None);
    }

    #[tokio::test]
    async fn spawn_runs_and_drop_kills_the_child() {
        let directory = tempfile::tempdir().expect("temp");
        let script = write_script(directory.path(), "slow-knectd", "#!/bin/sh\nsleep 30\n");
        let socket = directory.path().join("knectd.sock");
        let mut daemon = spawn_daemon(&script, &socket).await.expect("spawn");
        assert!(daemon.id().is_some());
        assert!(daemon.log_path().is_some());
        daemon.terminate();
        assert!(daemon.wait_for_exit(Duration::from_secs(2)).await);
    }
}
