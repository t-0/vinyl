//! Keyboard handling shared by the terminal and desktop front ends.
//!
//! Each front end translates its own key events into [`Key`] and calls
//! [`handle`], so both react to exactly the same keys in the same way.
use crate::app::App;
const SEEK_SECS: f64 = 5.0;
/// A key press, reduced to what the app cares about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// A printable character, as typed (so shift+e arrives as `'E'`).
    Char(char),
    Enter,
    Esc,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
}
/// Applies a key press to the app. `ctrl` is set while control (or the
/// platform/command key) is held.
pub fn handle(app: &mut App, key: Key, ctrl: bool) {
    if app.search_input.is_some() {
        handle_search_key(app, key, ctrl);
        return;
    }
    if app.fx_open {
        handle_fx_key(app, key);
        return;
    }
    if app.eq_open {
        handle_eq_key(app, key);
        return;
    }
    if app.sort_open {
        match key {
            Key::Esc | Key::Char('o') | Key::Char('q') => app.sort_open = false,
            Key::Enter | Key::Char(' ') => app.apply_sort(),
            Key::Up | Key::Char('k') => app.sort_move(-1),
            Key::Down | Key::Char('j') => app.sort_move(1),
            Key::Char('a') => {
                app.sort_row = 0;
                app.apply_sort()
            }
            Key::Char('t') => {
                app.sort_row = 1;
                app.apply_sort()
            }
            Key::Char('b') => {
                app.sort_row = 2;
                app.apply_sort()
            }
            Key::Char('r') => {
                app.sort_row = 3;
                app.apply_sort()
            }
            _ => {}
        }
        return;
    }
    match key {
        Key::Char('q') | Key::Esc => app.should_quit = true,
        Key::Char('c') if ctrl => app.should_quit = true,
        Key::Char('s') => app.section = crate::app::Section::Songs,
        Key::Char('a') => app.section = crate::app::Section::Albums,
        Key::Char('r') => app.section = crate::app::Section::Artists,
        Key::Char(' ') => app.toggle_play(),
        Key::Char('n') => app.step_track(1),
        Key::Char('p') => app.step_track(-1),
        Key::Up | Key::Char('k') => app.move_cursor(-1),
        Key::Down | Key::Char('j') => app.move_cursor(1),
        Key::Home => app.move_cursor(i64::MIN / 2),
        Key::End => app.move_cursor(i64::MAX / 2),
        Key::Left | Key::Char(',') => app.seek(-SEEK_SECS),
        Key::Right | Key::Char('.') => app.seek(SEEK_SECS),
        Key::Char('=') | Key::Char('+') => app.adjust_speed(1),
        Key::Char('-') | Key::Char('_') => app.adjust_speed(-1),
        Key::PageUp => app.move_cursor(-(app.page_size as i64)),
        Key::PageDown => app.move_cursor(app.page_size as i64),
        Key::Char('0') => app.reset_speed(),
        Key::Enter => app.activate_cursor(),
        Key::Char('/') => app.open_search(),
        Key::Char('x') => app.fx_open = true,
        Key::Char('e') => app.eq_open = true,
        Key::Char('o') => app.open_sort(),
        Key::Char('l') => app.toggle_loudness(),
        Key::Char('z') => app.toggle_shuffle(),
        Key::Char('f') => app.reveal_in_finder(),
        Key::Char('E') => app.edit_in_editor(),
        Key::Char('i') => app.start_index(true),
        _ => {}
    }
}
fn handle_search_key(app: &mut App, key: Key, ctrl: bool) {
    let Some(buf) = app.search_input.as_mut() else {
        return;
    };
    match key {
        Key::Esc => app.cancel_search(),
        Key::Enter => app.submit_search(),
        Key::Backspace => {
            buf.pop();
        }
        Key::Char('u') if ctrl => buf.clear(),
        Key::Char('c') if ctrl => app.cancel_search(),
        Key::Char(_) if ctrl => {}
        Key::Char(c) => buf.push(c),
        _ => {}
    }
}
fn handle_fx_key(app: &mut App, key: Key) {
    match key {
        Key::Esc | Key::Enter | Key::Char('x') | Key::Char('q') => app.close_fx(),
        Key::Up | Key::Char('k') => app.fx_move(-1),
        Key::Down | Key::Char('j') => app.fx_move(1),
        Key::Left | Key::Char('h') => app.fx_adjust(-5),
        Key::Right | Key::Char('l') => app.fx_adjust(5),
        Key::Char('-') | Key::Char('_') => app.fx_adjust(-1),
        Key::Char('=') | Key::Char('+') => app.fx_adjust(1),
        Key::Char(' ') => app.fx_adjust(5),
        Key::Char('0') => app.fx_reset_row(),
        _ => {}
    }
}
fn handle_eq_key(app: &mut App, key: Key) {
    match key {
        Key::Esc | Key::Enter | Key::Char('e') | Key::Char('q') => app.close_eq(),
        Key::Up | Key::Char('k') => app.eq_move(-1),
        Key::Down | Key::Char('j') => app.eq_move(1),
        Key::Left | Key::Char('h') | Key::Char('-') | Key::Char('_') => app.eq_adjust(-1),
        Key::Right | Key::Char('l') | Key::Char('=') | Key::Char('+') | Key::Char(' ') => {
            app.eq_adjust(1)
        }
        Key::PageUp => app.eq_adjust(3),
        Key::PageDown => app.eq_adjust(-3),
        Key::Char('0') => app.eq_reset_row(),
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Section;
    use crate::config::{Config, EditorMode, Ui};
    use crate::fx::Mode;
    fn app() -> App {
        let dir = std::env::temp_dir().join(format!("vinyl-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        App::new_for_test(Config {
            music_home: dir.clone(),
            index_file: dir.join("index.json"),
            audio_editor: None,
            audio_editor_mode: EditorMode::Gui,
            ui: Ui::Tui,
        })
    }
    #[test]
    fn main_screen_keys() {
        let mut a = app();
        handle(&mut a, Key::Char('a'), false);
        assert_eq!(a.section, Section::Albums);
        handle(&mut a, Key::Char('r'), false);
        assert_eq!(a.section, Section::Artists);
        handle(&mut a, Key::Char('s'), false);
        assert_eq!(a.section, Section::Songs);
        handle(&mut a, Key::Char('='), false);
        handle(&mut a, Key::Char('+'), false);
        assert_eq!(a.speed_pct, 2);
        handle(&mut a, Key::Char('0'), false);
        assert_eq!(a.speed_pct, 0);
        handle(&mut a, Key::Char('z'), false);
        assert!(a.shuffle);
        assert!(!a.should_quit);
        handle(&mut a, Key::Char('c'), true);
        assert!(a.should_quit);
    }
    #[test]
    fn dialogs_capture_keys() {
        let mut a = app();
        handle(&mut a, Key::Char('/'), false);
        assert!(a.search_input.is_some());
        for c in "q x".chars() {
            handle(&mut a, Key::Char(c), false);
        }
        // Typing into the search box must not quit or open FX.
        assert!(!a.should_quit && !a.fx_open);
        assert_eq!(a.search_input.as_deref(), Some("q x"));
        handle(&mut a, Key::Backspace, false);
        handle(&mut a, Key::Char('u'), true);
        assert_eq!(a.search_input.as_deref(), Some(""));
        handle(&mut a, Key::Esc, false);
        assert!(a.search_input.is_none() && !a.should_quit);
        handle(&mut a, Key::Char('x'), false);
        assert!(a.fx_open);
        handle(&mut a, Key::Right, false);
        assert_eq!(a.fx.mode, Mode::Vinyl);
        handle(&mut a, Key::Char('q'), false);
        assert!(!a.fx_open && !a.should_quit);
        handle(&mut a, Key::Char('e'), false);
        assert!(a.eq_open);
        handle(&mut a, Key::Esc, false);
        assert!(!a.eq_open);
        handle(&mut a, Key::Char('o'), false);
        assert!(a.sort_open);
        handle(&mut a, Key::Esc, false);
        assert!(!a.sort_open && !a.should_quit);
        handle(&mut a, Key::Esc, false);
        assert!(a.should_quit);
    }
}
