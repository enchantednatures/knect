//! Key mapping.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::actions::Action;
use crate::app::state::Screen;

/// Maps a key press to an action. Unmapped keys return `None`.
#[must_use]
pub fn map_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('1') => Some(Action::Switch(Screen::Graph)),
        KeyCode::Char('2') => Some(Action::Switch(Screen::Diff)),
        KeyCode::Char('3') => Some(Action::Switch(Screen::Tap)),
        KeyCode::Char('?') => Some(Action::Switch(Screen::Help)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrevious),
        KeyCode::Enter => Some(Action::OpenDetails),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn common_keys_map_to_actions() {
        assert_eq!(map_key(key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(map_key(key(KeyCode::Char('r'))), Some(Action::Refresh));
        assert_eq!(map_key(key(KeyCode::Down)), Some(Action::SelectNext));
        assert_eq!(
            map_key(key(KeyCode::Char('2'))),
            Some(Action::Switch(Screen::Diff))
        );
        assert_eq!(map_key(key(KeyCode::F(12))), None);
    }

    #[test]
    fn control_c_quits() {
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(event), Some(Action::Quit));
    }
}
