//! The terminal front end: ratatui drawing (see [`crate::ui`]) driven by a
//! crossterm event loop.
use crate::app::App;
use crate::config::{self, Config};
use crate::keys::{self, Key};
use crate::stderr;
use crate::ui;
use anyhow::Result;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use std::process::Command;
use std::time::{Duration, Instant};
/// How often the whole screen is rewritten cell by cell, so anything a
/// library or child process managed to print over the UI gets painted out.
const FULL_REPAINT: Duration = Duration::from_secs(5);
pub fn run(config: Config) -> Result<()> {
    // Anything written to stderr while the TUI owns the screen (native audio
    // libraries, child processes) would land on top of the UI, so send it to
    // a log file until the terminal is handed back.
    let _quiet = stderr::Redirect::to_file(&config::log_path());
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, App::new(config));
    ratatui::restore();
    result
}
fn event_loop(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    let mut last_tick = Instant::now();
    let mut last_repaint = Instant::now();
    loop {
        app.update_spectrum();
        let repaint = last_repaint.elapsed() >= FULL_REPAINT;
        let mut snapshot = None;
        terminal.draw(|f| {
            ui::draw(f, &mut app);
            if repaint {
                snapshot = Some(f.buffer_mut().clone());
            }
        })?;
        if let Some(buf) = snapshot {
            // ratatui only emits cells that changed since the last frame, so
            // stray text in otherwise static areas would survive forever.
            // Rewriting every cell (no clear, so no flicker) paints it out.
            let backend = terminal.backend_mut();
            backend.draw(buf.content.iter().enumerate().map(|(i, c)| {
                let (x, y) = buf.pos_of(i);
                (x, y, c)
            }))?;
            backend.flush()?;
            last_repaint = Instant::now();
        }
        let tick = if app.audio_active() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };
        let mut wait = tick.saturating_sub(last_tick.elapsed());
        while event::poll(wait)? {
            if let Event::Key(key) = event::read()?
                && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && let Some(k) = translate(key.code)
            {
                keys::handle(&mut app, k, key.modifiers.contains(KeyModifiers::CONTROL));
            }
            wait = Duration::ZERO;
            if app.should_quit {
                break;
            }
        }
        if let Some((name, cmd)) = app.pending_editor.take() {
            run_editor(&mut app, &name, cmd)?;
            // The alternate screen is blank again but ratatui's last frame is
            // not, so make the next frame rewrite every cell.
            last_repaint = Instant::now() - FULL_REPAINT;
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
fn translate(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        _ => return None,
    })
}
/// Runs a terminal audio editor in the foreground: playback is paused, the
/// screen, raw mode and stderr are handed back to the terminal for the
/// child's lifetime, then everything is put back and playback resumes.
fn run_editor(app: &mut App, name: &str, mut cmd: Command) -> Result<()> {
    let was_playing = app.audio_active();
    if was_playing && let Some(a) = &app.audio {
        a.pause();
    }
    ratatui::restore();
    let result = {
        let _stderr = stderr::suspend();
        cmd.status()
    };
    enable_raw_mode()?;
    // Not `Terminal::clear`: that asks the terminal where the cursor is and
    // waits for the answer, which an editor's leftover input can garble.
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        Clear(ClearType::All)
    )?;
    if was_playing && let Some(a) = &app.audio {
        a.resume();
    }
    app.editor_finished(name, result);
    Ok(())
}
