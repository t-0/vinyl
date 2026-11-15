//! Temporarily points the process's stderr (fd 2) at a file.
//!
//! The TUI owns the terminal while it runs, and nothing in this crate writes to
//! stderr then, but native libraries underneath the audio stack (CoreAudio,
//! cpal) occasionally do. Those bytes bypass ratatui entirely and end up as
//! garbage in the middle of the screen. Redirecting the file descriptor itself
//! catches every writer, not just Rust code.
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
/// Original stderr, kept so a panic message can still reach the user after
/// ratatui's panic hook has restored the terminal.
static SAVED_STDERR: AtomicI32 = AtomicI32::new(-1);
/// The log file, kept so the redirect can be put back after a `suspend`.
static LOG_FD: AtomicI32 = AtomicI32::new(-1);
pub struct Redirect {
    _private: (),
}
impl Redirect {
    /// Sends stderr to `path` (appending) until the guard is dropped. If the
    /// file cannot be opened or the descriptors cannot be duplicated, stderr
    /// is left alone; the UI then simply gets no protection.
    pub fn to_file(path: &Path) -> Option<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        // SAFETY: plain POSIX descriptor duplication on descriptors we own or
        // that are guaranteed open (2 is stderr). No memory is involved.
        unsafe {
            let saved = libc::dup(libc::STDERR_FILENO);
            if saved < 0 {
                return None;
            }
            if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
                libc::close(saved);
                return None;
            }
            SAVED_STDERR.store(saved, Ordering::SeqCst);
            LOG_FD.store(file.into_raw_fd(), Ordering::SeqCst);
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Some(Self { _private: () })
    }
}
impl Drop for Redirect {
    fn drop(&mut self) {
        restore();
    }
}
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
/// Hands the real stderr back to the process (and anything it spawns) while
/// the guard lives, e.g. for a child that takes over the terminal. Dropping
/// the guard sends stderr to the log file again. Does nothing when no
/// redirect is active.
pub struct Suspended {
    _private: (),
}
pub fn suspend() -> Suspended {
    let saved = SAVED_STDERR.load(Ordering::SeqCst);
    if saved >= 0 {
        // SAFETY: `saved` is an open descriptor owned by this module.
        unsafe {
            libc::dup2(saved, libc::STDERR_FILENO);
        }
    }
    Suspended { _private: () }
}
impl Drop for Suspended {
    fn drop(&mut self) {
        let log = LOG_FD.load(Ordering::SeqCst);
        if log >= 0 && SAVED_STDERR.load(Ordering::SeqCst) >= 0 {
            // SAFETY: `log` is an open descriptor owned by this module.
            unsafe {
                libc::dup2(log, libc::STDERR_FILENO);
            }
        }
    }
}
