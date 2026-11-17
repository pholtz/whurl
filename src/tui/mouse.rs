//! Mouse events (with `ui.mouse` on), routed by where the last frame drew each pane.

use super::app::{App, Focus, Mode};
use super::tree::Row;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

pub fn on_mouse(app: &mut App, m: MouseEvent) {
    let pos = Position::new(m.column, m.row);
    // Overlays and inputs: the wheel moves their selection like ↑↓; clicks are ignored.
    if app.mode != Mode::Normal {
        let code = match m.kind {
            MouseEventKind::ScrollDown => KeyCode::Down,
            MouseEventKind::ScrollUp => KeyCode::Up,
            _ => return,
        };
        return super::on_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }
    let in_sidebar = app.hits.sidebar.contains(pos);
    let in_main = app.hits.main.contains(pos);
    match m.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let d = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
            if in_sidebar {
                app.move_sel(d);
            } else if in_main {
                app.scroll_by(3 * d);
            }
        }
        MouseEventKind::ScrollRight if in_main => app.scroll_h(8),
        MouseEventKind::ScrollLeft if in_main => app.scroll_h(-8),
        // The sidebar's right border is a handle: drag it to resize.
        MouseEventKind::Down(MouseButton::Left) if in_sidebar && pos.x + 1 == app.hits.sidebar.right() => {
            app.dragging = true;
        }
        MouseEventKind::Drag(MouseButton::Left) if app.dragging => {
            app.sidebar_w = Some(pos.x.saturating_sub(app.hits.sidebar.x) + 1);
        }
        MouseEventKind::Up(MouseButton::Left) => app.dragging = false,
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(&(_, tab)) = app.hits.tabs.iter().find(|(r, _)| r.contains(pos)) {
                app.focus = Focus::Main;
                app.set_tab(tab);
            } else if app.hits.list.contains(pos) {
                app.focus = Focus::Sidebar;
                let i = app.list.offset() + (pos.y - app.hits.list.y) as usize;
                if i < app.rows.len() {
                    let cur = app.list.selected().unwrap_or(0) as isize;
                    app.move_sel(i as isize - cur);
                    // A folder opens or closes on click, like the arrow suggests.
                    if matches!(app.rows.get(i), Some(Row::Dir { .. })) {
                        app.toggle_dir();
                    }
                }
            } else if in_sidebar {
                app.focus = Focus::Sidebar;
            } else if in_main {
                app.focus = Focus::Main;
            }
        }
        _ => {}
    }
}
