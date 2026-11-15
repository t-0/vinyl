//! Temporarily points the process's stderr (fd 2) at a file.
//!
//! The TUI owns the terminal while it runs, and nothing in this crate writes to
//! stderr then, but native libraries underneath the audio stack (CoreAudio,
//! cpal) occasionally do. Those bytes bypass ratatui entirely and end up as
//! garbage in the middle of the screen. Redirecting the file descriptor itself
//! catches every writer, not just Rust code.
mod redirect;
mod suspended;
pub use redirect::Redirect;
use std::sync::atomic::{AtomicI32, Ordering};
pub use suspended::suspend;
/// Original stderr, kept so a panic message can still reach the user after
/// ratatui's panic hook has restored the terminal.
static SAVED_STDERR: AtomicI32 = AtomicI32::new(-1);
/// The log file, kept so the redirect can be put back after a `suspend`.
static LOG_FD: AtomicI32 = AtomicI32::new(-1);
/// Puts the original stderr back. Safe to call more than once.
fn restore() {
    let saved = SAVED_STDERR.swap(-1, Ordering::SeqCst);
    if saved >= 0 {
        // SAFETY: `saved` came from dup() above and has not been closed.
        unsafe {
            libc::dup2(saved, libc::STDERR_FILENO);
            libc::close(saved);
        }
    }
    let log = LOG_FD.swap(-1, Ordering::SeqCst);
    if log >= 0 {
        // SAFETY: `log` is the descriptor taken from the log file above.
        unsafe {
            libc::close(log);
        }
    }
}
