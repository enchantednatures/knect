//! Key mapping for normal navigation and the filter prompt.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::actions::Action;
use crate::app::modal::ModalKind;
use crate::app::state::InputMode;

/// Action within an open modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKey {
    /// Close the modal and clean up.
    Close,
    /// Reload the modal payload.
    Refresh,
    /// Select or scroll up.
    Up,
    /// Select or scroll down.
    Down,
    /// Decrement or move left.
    Left,
    /// Increment or move right.
    Right,
    /// Jump to the top.
    Top,
    /// Jump to the bottom.
    Bottom,
    /// Append a character to the active edit.
    Text(char),
    /// Remove the last character.
    Backspace,
    /// Commit the active edit.
    Commit,
    /// Discard the active edit without closing.
    CancelEdit,
    /// Begin editing a traffic tag.
    BeginTagEdit,
    /// Request a dry-run preview.
    Preview,
    /// Request (or confirm) a write.
    Submit,
}

/// Maps a key press while a modal is open.
#[must_use]
pub fn map_modal_key(key: KeyEvent, kind: ModalKind) -> Option<ModalKey> {
    match kind {
        ModalKind::None => None,
        ModalKind::Diff => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(ModalKey::Close),
            KeyCode::Char('r') => Some(ModalKey::Refresh),
            KeyCode::Up | KeyCode::Char('k') => Some(ModalKey::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(ModalKey::Down),
            KeyCode::Char('g') => Some(ModalKey::Top),
            KeyCode::Char('G') => Some(ModalKey::Bottom),
            _ => None,
        },
        ModalKind::Traffic => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(ModalKey::Close),
            KeyCode::Char('r') => Some(ModalKey::Refresh),
            KeyCode::Up | KeyCode::Char('k') => Some(ModalKey::Up),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => Some(ModalKey::Down),
            KeyCode::BackTab => Some(ModalKey::Up),
            KeyCode::Left | KeyCode::Char('-') => Some(ModalKey::Left),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => Some(ModalKey::Right),
            KeyCode::Char('g') => Some(ModalKey::Top),
            KeyCode::Char('G') => Some(ModalKey::Bottom),
            KeyCode::Char('t') => Some(ModalKey::BeginTagEdit),
            KeyCode::Char('p') => Some(ModalKey::Preview),
            KeyCode::Char('s') => Some(ModalKey::Submit),
            KeyCode::Enter => Some(ModalKey::Commit),
            KeyCode::Backspace => Some(ModalKey::Backspace),
            KeyCode::Char(character @ '0'..='9') => Some(ModalKey::Text(character)),
            _ => None,
        },
        ModalKind::Tap => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(ModalKey::Close),
            KeyCode::Up | KeyCode::Char('k') => Some(ModalKey::Up),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => Some(ModalKey::Down),
            KeyCode::Char('g') => Some(ModalKey::Top),
            KeyCode::Char('G') => Some(ModalKey::Bottom),
            _ => None,
        },
    }
}

/// Maps a key press while a traffic percentage or tag is being edited.
#[must_use]
pub fn map_modal_text_key(key: KeyEvent) -> Option<ModalKey> {
    match key.code {
        KeyCode::Esc => Some(ModalKey::CancelEdit),
        KeyCode::Enter | KeyCode::Tab => Some(ModalKey::Commit),
        KeyCode::Backspace => Some(ModalKey::Backspace),
        KeyCode::Char(character) => Some(ModalKey::Text(character)),
        _ => None,
    }
}

/// Context needed to disambiguate multi-key sequences and input modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyContext {
    /// Current input mode.
    pub mode: InputMode,
    /// A previous `g` is waiting for its second key.
    pub pending_g: bool,
}

/// Maps a key press to an action. Unmapped keys return `None`.
#[must_use]
pub fn map_key(key: KeyEvent, context: KeyContext) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    if context.mode == InputMode::Filter {
        return match key.code {
            KeyCode::Esc => Some(Action::FilterCancel),
            KeyCode::Enter => Some(Action::FilterCommit),
            KeyCode::Backspace => Some(Action::FilterBackspace),
            KeyCode::Char(character) => Some(Action::FilterChar(character)),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Esc => Some(Action::CloseOverlay),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char('/') => Some(Action::BeginFilter),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char(digit @ '1'..='4') => digit
            .to_digit(10)
            .and_then(|value| u8::try_from(value).ok())
            .map(Action::InspectorTab),
        KeyCode::Tab => Some(Action::PaneNext),
        KeyCode::BackTab => Some(Action::PanePrevious),
        KeyCode::Enter => Some(Action::FocusDetails),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::Left),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::Right),
        KeyCode::Char('G') => Some(Action::Bottom),
        KeyCode::Char('g') => Some(if context.pending_g {
            Action::Top
        } else {
            Action::PendingG
        }),
        KeyCode::Char('d') => Some(if context.pending_g {
            Action::FocusDetails
        } else {
            Action::OpenDiff
        }),
        KeyCode::Char('t') => Some(Action::OpenTraffic),
        KeyCode::Char('w') => Some(Action::OpenTap),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn normal() -> KeyContext {
        KeyContext::default()
    }

    #[test]
    fn navigation_keys_map_to_actions() {
        assert_eq!(
            map_key(key(KeyCode::Char('j')), normal()),
            Some(Action::Down)
        );
        assert_eq!(map_key(key(KeyCode::Down), normal()), Some(Action::Down));
        assert_eq!(map_key(key(KeyCode::Char('k')), normal()), Some(Action::Up));
        assert_eq!(
            map_key(key(KeyCode::Char('h')), normal()),
            Some(Action::Left)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('l')), normal()),
            Some(Action::Right)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('G')), normal()),
            Some(Action::Bottom)
        );
    }

    #[test]
    fn g_sequences_map_to_top_and_details() {
        assert_eq!(
            map_key(key(KeyCode::Char('g')), normal()),
            Some(Action::PendingG)
        );
        let pending = KeyContext {
            pending_g: true,
            ..KeyContext::default()
        };
        assert_eq!(map_key(key(KeyCode::Char('g')), pending), Some(Action::Top));
        assert_eq!(
            map_key(key(KeyCode::Char('d')), pending),
            Some(Action::FocusDetails)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('d')), normal()),
            Some(Action::OpenDiff)
        );
    }

    #[test]
    fn panes_tabs_and_overlays_map() {
        assert_eq!(map_key(key(KeyCode::Tab), normal()), Some(Action::PaneNext));
        assert_eq!(
            map_key(key(KeyCode::BackTab), normal()),
            Some(Action::PanePrevious)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('1')), normal()),
            Some(Action::InspectorTab(1))
        );
        assert_eq!(
            map_key(key(KeyCode::Char('4')), normal()),
            Some(Action::InspectorTab(4))
        );
        assert_eq!(
            map_key(key(KeyCode::Char('?')), normal()),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('/')), normal()),
            Some(Action::BeginFilter)
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), normal()),
            Some(Action::CloseOverlay)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('q')), normal()),
            Some(Action::Quit)
        );
    }

    #[test]
    fn filter_mode_edits_text_not_navigation() {
        let filter = KeyContext {
            mode: InputMode::Filter,
            pending_g: false,
        };
        assert_eq!(
            map_key(key(KeyCode::Char('j')), filter),
            Some(Action::FilterChar('j'))
        );
        assert_eq!(
            map_key(key(KeyCode::Backspace), filter),
            Some(Action::FilterBackspace)
        );
        assert_eq!(
            map_key(key(KeyCode::Enter), filter),
            Some(Action::FilterCommit)
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), filter),
            Some(Action::FilterCancel)
        );
    }

    #[test]
    fn modal_keys_open_the_dashboards() {
        assert_eq!(
            map_key(key(KeyCode::Char('d')), normal()),
            Some(Action::OpenDiff)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('t')), normal()),
            Some(Action::OpenTraffic)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('w')), normal()),
            Some(Action::OpenTap)
        );
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(control_c, normal()), Some(Action::Quit));
        assert_eq!(map_key(key(KeyCode::F(12)), normal()), None);
    }

    #[test]
    fn diff_modal_keys_navigate_and_close() {
        assert_eq!(
            map_modal_key(key(KeyCode::Char('j')), ModalKind::Diff),
            Some(ModalKey::Down)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('r')), ModalKind::Diff),
            Some(ModalKey::Refresh)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Esc), ModalKind::Diff),
            Some(ModalKey::Close)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('p')), ModalKind::Diff),
            None
        );
    }

    #[test]
    fn traffic_modal_keys_edit_and_submit() {
        assert_eq!(
            map_modal_key(key(KeyCode::Char('+')), ModalKind::Traffic),
            Some(ModalKey::Right)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('5')), ModalKind::Traffic),
            Some(ModalKey::Text('5'))
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('p')), ModalKind::Traffic),
            Some(ModalKey::Preview)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('s')), ModalKind::Traffic),
            Some(ModalKey::Submit)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('t')), ModalKind::Traffic),
            Some(ModalKey::BeginTagEdit)
        );
        assert_eq!(
            map_modal_text_key(key(KeyCode::Char('p'))),
            Some(ModalKey::Text('p'))
        );
        assert_eq!(
            map_modal_text_key(key(KeyCode::Esc)),
            Some(ModalKey::CancelEdit)
        );
        assert_eq!(
            map_modal_text_key(key(KeyCode::Enter)),
            Some(ModalKey::Commit)
        );
    }

    #[test]
    fn tap_modal_keys_scroll_and_close() {
        assert_eq!(
            map_modal_key(key(KeyCode::Char('k')), ModalKind::Tap),
            Some(ModalKey::Up)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('G')), ModalKind::Tap),
            Some(ModalKey::Bottom)
        );
        assert_eq!(
            map_modal_key(key(KeyCode::Char('q')), ModalKind::Tap),
            Some(ModalKey::Close)
        );
    }
}
