use super::{LOG_FD, SAVED_STDERR};
use std::sync::atomic::Ordering;
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
