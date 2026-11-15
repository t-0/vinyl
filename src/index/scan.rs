use super::Track;
use std::path::PathBuf;
pub struct Scan {
    pub tracks: Vec<Track>,
    pub unreadable: Vec<PathBuf>,
}
