mod app;
mod audio;
mod config;
mod fx;
mod index;
mod spectrum;
mod ui;
use anyhow::Result;
use app::App;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};
const SEEK_SECS: f64 = 5.0;
fn main() -> Result<()> {
    let config = config::load()?;
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, App::new(config));
    ratatui::restore();
    result
}
fn run(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        app.update_spectrum();
        terminal.draw(|f| ui::draw(f, &mut app))?;
        let tick = if app.audio_active() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };
        let mut wait = tick.saturating_sub(last_tick.elapsed());
        while event::poll(wait)? {
            if let Event::Key(key) = event::read()?
                && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            {
                handle_key(&mut app, key.code, key.modifiers);
            }
            wait = Duration::ZERO;
            if app.should_quit {
                break;
            }
        }
        if last_tick.elapsed() >= tick {
            app.tick()?;
            last_tick = Instant::now();
        }
        app.flush_play();
        if app.should_quit {
            return Ok(());
        }
    }
}
fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if app.search_input.is_some() {
        handle_search_key(app, code, mods);
        return;
    }
    if app.fx_open {
        handle_fx_key(app, code);
        return;
    }
    if app.eq_open {
        handle_eq_key(app, code);
        return;
    }
    if app.sort_open {
        match code {
            KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('q') => app.sort_open = false,
            KeyCode::Enter | KeyCode::Char(' ') => app.apply_sort(),
            KeyCode::Up | KeyCode::Char('k') => app.sort_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.sort_move(1),
            KeyCode::Char('a') => {
                app.sort_row = 0;
                app.apply_sort()
            }
            KeyCode::Char('t') => {
                app.sort_row = 1;
                app.apply_sort()
            }
            KeyCode::Char('b') => {
                app.sort_row = 2;
                app.apply_sort()
            }
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
        KeyCode::Char('s') => app.section = app::Section::Songs,
        KeyCode::Char('a') => app.section = app::Section::Albums,
        KeyCode::Char('r') => app.section = app::Section::Artists,
        KeyCode::Char(' ') => app.toggle_play(),
        KeyCode::Char('n') => app.step_track(1),
        KeyCode::Char('p') => app.step_track(-1),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
        KeyCode::Home => app.move_cursor(i64::MIN / 2),
        KeyCode::End => app.move_cursor(i64::MAX / 2),
        KeyCode::Left | KeyCode::Char(',') => app.seek(-SEEK_SECS),
        KeyCode::Right | KeyCode::Char('.') => app.seek(SEEK_SECS),
        KeyCode::Char('=') | KeyCode::Char('+') => app.adjust_speed(1),
        KeyCode::Char('-') | KeyCode::Char('_') => app.adjust_speed(-1),
        KeyCode::PageUp => app.move_cursor(-(app.page_size as i64)),
        KeyCode::PageDown => app.move_cursor(app.page_size as i64),
        KeyCode::Char('0') => app.reset_speed(),
        KeyCode::Enter => app.activate_cursor(),
        KeyCode::Char('/') => app.open_search(),
        KeyCode::Char('v') => app.fx_open = true,
        KeyCode::Char('e') => app.eq_open = true,
        KeyCode::Char('o') => app.open_sort(),
        KeyCode::Char('l') => app.toggle_loudness(),
        KeyCode::Char('f') => app.reveal_in_finder(),
        KeyCode::Char('i') => app.start_index(true),
        _ => {}
    }
}
fn handle_search_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let Some(buf) = app.search_input.as_mut() else {
        return;
    };
    match code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.submit_search(),
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => buf.clear(),
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.cancel_search(),
        KeyCode::Char(c) => buf.push(c),
        _ => {}
    }
}
fn handle_fx_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('v') | KeyCode::Char('q') => app.close_fx(),
        KeyCode::Up | KeyCode::Char('k') => app.fx_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.fx_move(1),
        KeyCode::Left | KeyCode::Char('h') => app.fx_adjust(-5),
        KeyCode::Right | KeyCode::Char('l') => app.fx_adjust(5),
        KeyCode::Char('-') | KeyCode::Char('_') => app.fx_adjust(-1),
        KeyCode::Char('=') | KeyCode::Char('+') => app.fx_adjust(1),
        KeyCode::Char(' ') => app.fx_adjust(5),
        KeyCode::Char('0') => app.fx_reset_row(),
        _ => {}
    }
}
fn handle_eq_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('q') => app.close_eq(),
        KeyCode::Up | KeyCode::Char('k') => app.eq_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.eq_move(1),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') | KeyCode::Char('_') => {
            app.eq_adjust(-1)
        }
        KeyCode::Right
        | KeyCode::Char('l')
        | KeyCode::Char('=')
        | KeyCode::Char('+')
        | KeyCode::Char(' ') => app.eq_adjust(1),
        KeyCode::PageUp => app.eq_adjust(3),
        KeyCode::PageDown => app.eq_adjust(-3),
        KeyCode::Char('0') => app.eq_reset_row(),
        _ => {}
    }
}
