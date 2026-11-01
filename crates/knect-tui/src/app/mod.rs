//! Application state, key handling, and the async event loop.

pub mod actions;
pub mod event;
pub mod keys;
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use knect_proto::rpc::{
    DaemonStatus, DaemonStatusParams, GetNodeDetailsParams, GetTopologyParams, LogsSubscribeParams,
    LogsSubscribeResult, methods,
};
use knect_proto::{NodeDetails, ResourceId, Topology};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{
    self as crossterm_event, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::actions::{Action, Effect, apply_action, selection_effects};
use crate::app::event::UiEvent;
use crate::app::keys::{KeyContext, map_key};
use crate::app::state::{AppState, ConnectionStatus};
use crate::client::{
    ClientError, ConnectOptions, Connected, RpcClient, SpawnedDaemon, connect, resolve_endpoint,
};
use crate::ui;

pub use actions::Action as UiAction;
pub use event::UiEvent as AppEvent;
pub use state::{AppState as State, Pane};

const TICK: Duration = Duration::from_millis(16);
const INITIAL_LOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// Application failure.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Client construction or endpoint resolution failed.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// Terminal setup, drawing, or restore failed.
    #[error("terminal error: {0}")]
    Io(#[from] std::io::Error),
}

/// Launches the dashboard.
pub async fn run(
    socket: Option<PathBuf>,
    daemon_url: Option<String>,
    ascii: bool,
    focus: Option<ResourceId>,
) -> Result<(), AppError> {
    let explicit = socket.is_some() || daemon_url.is_some();
    let endpoint = resolve_endpoint(socket.as_deref(), daemon_url.as_deref())?;
    let connected = connect(ConnectOptions::new(endpoint, explicit)).await?;
    let mut state = AppState::with_ascii(ascii);
    state.set_endpoint(connected.endpoint.describe());
    state.set_connection(if connected.reachable {
        ConnectionStatus::Connected
    } else {
        ConnectionStatus::Connecting
    });
    if let Some(warning) = &connected.warning {
        state.set_status(warning.clone());
    }
    let Connected {
        client, spawned, ..
    } = connected;
    run_tui(state, client, spawned, focus).await
}

/// Runs the event loop against an already-constructed client.
pub async fn run_tui(
    mut state: AppState,
    client: Arc<dyn RpcClient>,
    _daemon: Option<SpawnedDaemon>,
    focus: Option<ResourceId>,
) -> Result<(), AppError> {
    install_panic_hook();
    let mut session = TerminalSession::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    match tokio::time::timeout(
        INITIAL_LOAD_TIMEOUT,
        load_initial(&mut state, client.as_ref()),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(message)) => {
            state.set_connection(ConnectionStatus::Disconnected(message.clone()));
            state.set_error(message);
        }
        Err(_) => {
            state.set_connection(ConnectionStatus::Disconnected(
                "initial load timed out".to_owned(),
            ));
            state.set_error("daemon did not answer the initial status/topology calls");
        }
    }
    if let Some(id) = focus {
        state.select_resource(&id);
    }
    let initial_effects = selection_effects(&state);

    let result = run_loop(&mut terminal, &mut state, client, initial_effects).await;
    let _ = terminal.show_cursor();
    session.restore();
    result
}

async fn load_initial(state: &mut AppState, client: &dyn RpcClient) -> Result<(), String> {
    let status_params = serde_json::to_value(DaemonStatusParams::default())
        .map_err(|error| format!("cannot encode status params: {error}"))?;
    let topology_params = serde_json::to_value(GetTopologyParams::default())
        .map_err(|error| format!("cannot encode topology params: {error}"))?;
    let (status, topology) = tokio::join!(
        client.call(methods::DAEMON_STATUS.to_owned(), Some(status_params)),
        client.call(
            methods::GRAPH_GET_TOPOLOGY.to_owned(),
            Some(topology_params)
        ),
    );
    let mut errors = Vec::new();
    match status {
        Ok(value) => match serde_json::from_value::<DaemonStatus>(value) {
            Ok(status) => state.daemon = Some(status),
            Err(error) => errors.push(format!("daemon.status: {error}")),
        },
        Err(error) => errors.push(format!("daemon.status: {error}")),
    }
    match topology {
        Ok(value) => match serde_json::from_value::<Topology>(value) {
            Ok(topology) => state.set_topology(topology),
            Err(error) => errors.push(format!("graph.get_topology: {error}")),
        },
        Err(error) => errors.push(format!("graph.get_topology: {error}")),
    }
    if errors.is_empty() {
        state.set_connection(ConnectionStatus::Connected);
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut AppState,
    client: Arc<dyn RpcClient>,
    initial_effects: Vec<Effect>,
) -> Result<(), AppError> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<UiEvent>();
    spawn_input_thread(sender.clone());
    let cancel = CancellationToken::new();
    tokio::spawn(subscription_loop(
        Arc::clone(&client),
        sender.clone(),
        cancel.clone(),
    ));

    for effect in initial_effects {
        spawn_effect(Arc::clone(&client), sender.clone(), effect);
    }

    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    state.mark_dirty();
    terminal.draw(|frame| ui::render(frame, state))?;

    loop {
        let event = tokio::select! {
            received = receiver.recv() => match received {
                Some(event) => event,
                None => break,
            },
            _ = ticker.tick() => UiEvent::Tick,
            _ = tokio::signal::ctrl_c() => UiEvent::Quit,
        };
        let effects = handle_event(state, event);
        if state.should_quit {
            break;
        }
        for effect in effects {
            spawn_effect(Arc::clone(&client), sender.clone(), effect);
        }
        if state.take_dirty() {
            terminal.draw(|frame| ui::render(frame, state))?;
        }
    }
    cancel.cancel();
    Ok(())
}

fn handle_event(state: &mut AppState, event: UiEvent) -> Vec<Effect> {
    match event {
        UiEvent::Key(key) => handle_key(state, key),
        UiEvent::Tick => Vec::new(),
        UiEvent::Resize(_, _) => {
            state.mark_dirty();
            Vec::new()
        }
        UiEvent::Topology(topology) => {
            state.set_topology(*topology);
            Vec::new()
        }
        UiEvent::Status(status) => {
            state.daemon = Some(*status);
            state.set_connection(ConnectionStatus::Connected);
            Vec::new()
        }
        UiEvent::Details { id, result } => {
            if state.selected.as_ref() == Some(&id) {
                match result {
                    Ok(details) => {
                        state.details = Some(*details);
                        state.details_error = None;
                    }
                    Err(error) => {
                        state.details = None;
                        state.details_error = Some(error);
                    }
                }
                state.mark_dirty();
            }
            Vec::new()
        }
        UiEvent::Daemon(event) => {
            state.apply_event(*event);
            Vec::new()
        }
        UiEvent::Connection { status, message } => {
            let connected = status.is_connected();
            state.set_connection(status);
            if connected {
                state.error_banner = None;
            }
            if let Some(message) = message {
                if connected {
                    state.set_status(message);
                } else {
                    state.set_error(message);
                }
            }
            Vec::new()
        }
        UiEvent::LogsSubscribed { subscription } => {
            state.log_subscription = Some(subscription);
            state.set_status("log subscription active");
            Vec::new()
        }
        UiEvent::Notice { message, error } => {
            if error {
                state.set_error(message);
            } else {
                state.set_status(message);
            }
            Vec::new()
        }
        UiEvent::Quit => {
            state.quit();
            Vec::new()
        }
    }
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let context = KeyContext {
        mode: state.input_mode,
        pending_g: state.pending_g,
    };
    let action = map_key(key, context);
    let was_pending = state.pending_g;
    if was_pending {
        state.pending_g = false;
    }
    match action {
        Some(Action::PendingG) => {
            state.pending_g = true;
            Vec::new()
        }
        Some(action) => apply_action(state, action),
        None => Vec::new(),
    }
}

fn spawn_input_thread(sender: mpsc::UnboundedSender<UiEvent>) {
    std::thread::spawn(move || {
        loop {
            if sender.is_closed() {
                break;
            }
            match crossterm_event::poll(Duration::from_millis(100)) {
                Ok(true) => match crossterm_event::read() {
                    Ok(Event::Key(key)) => {
                        if sender.send(UiEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Resize(width, height)) => {
                        if sender.send(UiEvent::Resize(width, height)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
}

fn spawn_effect(
    client: Arc<dyn RpcClient>,
    sender: mpsc::UnboundedSender<UiEvent>,
    effect: Effect,
) {
    match effect {
        Effect::Refresh => {
            tokio::spawn(async move {
                let params = match serde_json::to_value(GetTopologyParams::default()) {
                    Ok(params) => params,
                    Err(error) => {
                        send_notice(&sender, format!("refresh params: {error}"), true);
                        return;
                    }
                };
                match client
                    .call(methods::GRAPH_GET_TOPOLOGY.to_owned(), Some(params))
                    .await
                {
                    Ok(value) => match serde_json::from_value::<Topology>(value) {
                        Ok(topology) => {
                            let _ = sender.send(UiEvent::Topology(Box::new(topology)));
                        }
                        Err(error) => {
                            send_notice(&sender, format!("refresh topology decode: {error}"), true)
                        }
                    },
                    Err(error) => send_notice(&sender, format!("refresh: {error}"), true),
                }
                let params = serde_json::to_value(DaemonStatusParams::default()).ok();
                if let Ok(value) = client.call(methods::DAEMON_STATUS.to_owned(), params).await
                    && let Ok(status) = serde_json::from_value::<DaemonStatus>(value)
                {
                    let _ = sender.send(UiEvent::Status(Box::new(status)));
                }
            });
        }
        Effect::FetchDetails(id) => {
            tokio::spawn(async move {
                let params = GetNodeDetailsParams {
                    id: id.clone(),
                    include_yaml: Some(true),
                    include_logs: Some(false),
                };
                let params = match serde_json::to_value(params) {
                    Ok(params) => params,
                    Err(error) => {
                        let _ = sender.send(UiEvent::Details {
                            id,
                            result: Err(format!("details params: {error}")),
                        });
                        return;
                    }
                };
                let result = match client
                    .call(methods::GRAPH_GET_NODE_DETAILS.to_owned(), Some(params))
                    .await
                {
                    Ok(value) => serde_json::from_value::<NodeDetails>(value)
                        .map(Box::new)
                        .map_err(|error| format!("details decode: {error}")),
                    Err(error) => Err(error.to_string()),
                };
                let _ = sender.send(UiEvent::Details { id, result });
            });
        }
        Effect::SubscribeLogs(pods) => {
            tokio::spawn(async move {
                let params = LogsSubscribeParams {
                    pods,
                    tail_lines: Some(200),
                    since: None,
                    filter: None,
                };
                let params = match serde_json::to_value(params) {
                    Ok(params) => params,
                    Err(error) => {
                        send_notice(&sender, format!("log params: {error}"), false);
                        return;
                    }
                };
                match client
                    .call(methods::LOGS_SUBSCRIBE.to_owned(), Some(params))
                    .await
                {
                    Ok(value) => match serde_json::from_value::<LogsSubscribeResult>(value) {
                        Ok(result) => {
                            let _ = sender.send(UiEvent::LogsSubscribed {
                                subscription: result.subscription_id,
                            });
                        }
                        Err(error) => {
                            send_notice(&sender, format!("log subscription decode: {error}"), false)
                        }
                    },
                    Err(error) => send_notice(&sender, format!("logs: {error}"), false),
                }
            });
        }
    }
}

fn send_notice(sender: &mpsc::UnboundedSender<UiEvent>, message: String, error: bool) {
    let _ = sender.send(UiEvent::Notice { message, error });
}

async fn subscription_loop(
    client: Arc<dyn RpcClient>,
    sender: mpsc::UnboundedSender<UiEvent>,
    cancel: CancellationToken,
) {
    let mut last_event_id = None;
    let mut attempt = 0_u32;
    loop {
        if cancel.is_cancelled() {
            break;
        }
        match client.subscribe(None, last_event_id).await {
            Ok(mut stream) => {
                attempt = 0;
                if sender
                    .send(UiEvent::Connection {
                        status: ConnectionStatus::Connected,
                        message: None,
                    })
                    .is_err()
                {
                    break;
                }
                loop {
                    let next = tokio::select! {
                        _ = cancel.cancelled() => return,
                        next = stream.next() => next,
                    };
                    match next {
                        Some(Ok(event)) => {
                            last_event_id = Some(event.seq);
                            if sender.send(UiEvent::Daemon(Box::new(event))).is_err() {
                                return;
                            }
                        }
                        Some(Err(error)) => {
                            let _ = sender.send(UiEvent::Connection {
                                status: ConnectionStatus::Reconnecting {
                                    attempt: attempt.saturating_add(1),
                                },
                                message: Some(error.to_string()),
                            });
                            break;
                        }
                        None => {
                            let _ = sender.send(UiEvent::Connection {
                                status: ConnectionStatus::Reconnecting {
                                    attempt: attempt.saturating_add(1),
                                },
                                message: Some("event stream ended".to_owned()),
                            });
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(UiEvent::Connection {
                    status: ConnectionStatus::Reconnecting {
                        attempt: attempt.saturating_add(1),
                    },
                    message: Some(error.to_string()),
                });
            }
        }
        attempt = attempt.saturating_add(1);
        let delay = Duration::from_millis(u64::from(attempt.min(20)) * 250);
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// Guard that restores terminal state exactly once.
pub struct RestoreGuard<F: FnMut()> {
    restore: F,
    restored: bool,
}

impl<F: FnMut()> RestoreGuard<F> {
    /// Creates a guard around a restore callback.
    pub fn new(restore: F) -> Self {
        Self {
            restore,
            restored: false,
        }
    }

    /// Runs the restore callback if it has not run yet.
    pub fn restore(&mut self) {
        if !self.restored {
            self.restored = true;
            (self.restore)();
        }
    }
}

impl<F: FnMut()> Drop for RestoreGuard<F> {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Raw-mode and alternate-screen session for the dashboard.
pub struct TerminalSession {
    guard: RestoreGuard<fn()>,
}

impl TerminalSession {
    /// Enters raw mode and the alternate screen.
    pub fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self {
            guard: RestoreGuard::new(restore_terminal as fn()),
        })
    }

    /// Restores the terminal immediately.
    pub fn restore(&mut self) {
        self.guard.restore();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    );
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn restore_guard_runs_once_on_an_error_path() {
        let count = Arc::new(AtomicUsize::new(0));
        {
            let counter = Arc::clone(&count);
            let mut guard = RestoreGuard::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            });
            let result: Result<(), &str> = Err("boom");
            assert!(result.is_err());
            guard.restore();
            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn gd_focuses_the_inspector_through_the_event_loop() {
        let mut state = AppState::new();
        handle_key(
            &mut state,
            KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('g'),
                KeyModifiers::NONE,
            ),
        );
        assert!(state.pending_g);
        handle_key(
            &mut state,
            KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('d'),
                KeyModifiers::NONE,
            ),
        );
        assert!(!state.pending_g);
        assert_eq!(state.active_pane, crate::app::state::Pane::Inspector);
    }

    #[test]
    fn release_events_are_ignored() {
        let mut state = AppState::new();
        let mut key = KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('q'),
            KeyModifiers::NONE,
        );
        key.kind = KeyEventKind::Release;
        handle_key(&mut state, key);
        assert!(!state.should_quit);
    }
}
