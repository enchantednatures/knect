//! Key mapping for normal navigation and the filter prompt.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::actions::Action;
use crate::app::state::InputMode;

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
            Action::Reserved("mutation diff")
        }),
        KeyCode::Char('t') => Some(Action::Reserved("traffic split")),
        KeyCode::Char('w') => Some(Action::Reserved("tap pane")),
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
            Some(Action::Reserved("mutation diff"))
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
    fn reserved_keys_and_control_c() {
        let reserved = [
            (KeyCode::Char('d'), "mutation diff"),
            (KeyCode::Char('t'), "traffic split"),
            (KeyCode::Char('w'), "tap pane"),
        ];
        for (code, feature) in reserved {
            assert_eq!(
                map_key(key(code), normal()),
                Some(Action::Reserved(feature))
            );
        }
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(control_c, normal()), Some(Action::Quit));
        assert_eq!(map_key(key(KeyCode::F(12)), normal()), None);
    }
}
