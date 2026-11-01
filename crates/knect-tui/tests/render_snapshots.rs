#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use knect_tui::app::state::AppState;
use knect_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn draw_text(state: &AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        output.push_str(line.trim_end());
        output.push('\n');
    }
    output
}

#[test]
fn dashboard_snapshot_at_80x24() {
    let state = support::snapshot_state();
    insta::assert_snapshot!("dashboard_80x24", draw_text(&state, 80, 24));
}

#[test]
fn dashboard_snapshot_at_160x50() {
    let state = support::snapshot_state();
    insta::assert_snapshot!("dashboard_160x50", draw_text(&state, 160, 50));
}

#[test]
fn help_overlay_snapshot() {
    let mut state = support::snapshot_state();
    state.help_visible = true;
    insta::assert_snapshot!("help_80x24", draw_text(&state, 80, 24));
}
