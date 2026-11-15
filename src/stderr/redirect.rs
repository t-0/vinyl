use super::{LOG_FD, SAVED_STDERR, restore};
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::path::Path;
use std::sync::atomic::Ordering;
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
