use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub mtime: u64,
    pub size: u64,
    pub artist: String,
    pub album_artist: Option<String>,
    pub album: String,
    pub title: String,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub duration_secs: f64,
    pub filetype: String,
    /// Auto-normalisation gain in dB, measured the first time the track is
    /// played with normalisation on (see `audio::measure_gain`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}
impl Track {
    pub fn group_artist(&self) -> &str {
        self.album_artist
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.artist)
    }
    pub fn sort_key(&self) -> (String, String, u32, u32, String) {
        (
            self.group_artist().to_lowercase(),
            self.album.to_lowercase(),
            self.disc.unwrap_or(0),
            self.track.unwrap_or(0),
            self.title.to_lowercase(),
        )
    }
}
