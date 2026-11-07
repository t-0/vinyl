use anyhow::{Context, Result};
use lofty::file::TaggedFileExt;
use lofty::prelude::*;
use lofty::tag::ItemKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "wav", "wave", "aif", "aiff", "wv",
    "ape", "mpc",
];
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub tracks: Vec<Track>,
}
pub enum IndexMsg {
    Progress { done: usize, total: usize },
    Done(Result<Scan>),
}
pub struct Scan {
    pub tracks: Vec<Track>,
    pub unreadable: Vec<PathBuf>,
}
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
fn load_existing(index_file: &Path) -> HashMap<PathBuf, Track> {
    let Ok(text) = fs::read_to_string(index_file) else {
        return HashMap::new();
    };
    let Ok(idx) = serde_json::from_str::<Index>(&text) else {
        return HashMap::new();
    };
    idx.tracks
        .into_iter()
        .map(|t| (t.path.clone(), t))
        .collect()
}
fn fallback_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}
fn fallback_album(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Album")
        .to_string()
}
fn read_track(path: &Path, mtime: u64, size: u64) -> Track {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut t = Track {
        path: path.to_path_buf(),
        mtime,
        size,
        artist: "Unknown Artist".into(),
        album_artist: None,
        album: fallback_album(path),
        title: fallback_title(path),
        year: None,
        track: None,
        disc: None,
        duration_secs: 0.0,
        filetype: ext.clone(),
        gain_db: None,
    };
    let tagged = match lofty::read_from_path(path) {
        Ok(f) => f,
        Err(_) => return t,
    };
    t.duration_secs = tagged.properties().duration().as_secs_f64();
    t.filetype = match tagged.file_type() {
        lofty::file::FileType::Mpeg => "mp3".into(),
        lofty::file::FileType::Flac => "flac".into(),
        lofty::file::FileType::Mp4 => {
            if ext == "m4b" {
                "m4b".into()
            } else {
                "m4a".into()
            }
        }
        lofty::file::FileType::Aac => "aac".into(),
        lofty::file::FileType::Vorbis => "ogg".into(),
        lofty::file::FileType::Opus => "opus".into(),
        lofty::file::FileType::Wav => "wav".into(),
        lofty::file::FileType::Aiff => "aiff".into(),
        lofty::file::FileType::Ape => "ape".into(),
        lofty::file::FileType::WavPack => "wv".into(),
        lofty::file::FileType::Mpc => "mpc".into(),
        _ => ext.clone(),
    };
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        let clean = |s: Option<std::borrow::Cow<'_, str>>| {
            s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        };
        if let Some(a) = clean(tag.artist()) {
            t.artist = a;
        }
        if let Some(a) = clean(tag.album()) {
            t.album = a;
        }
        if let Some(a) = clean(tag.title()) {
            t.title = a;
        }
        t.album_artist = tag
            .get_string(ItemKey::AlbumArtist)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        t.track = tag.track();
        t.disc = tag.disk();
        t.year = tag.date().map(|d| d.year as u32).or_else(|| {
            [
                ItemKey::Year,
                ItemKey::RecordingDate,
                ItemKey::OriginalReleaseDate,
            ]
            .into_iter()
            .filter_map(|k| tag.get_string(k))
            .filter_map(|s| s.chars().take(4).collect::<String>().parse::<u32>().ok())
            .next()
        });
    }
    t
}
pub fn save(index_file: &Path, tracks: &[Track]) -> Result<()> {
    let json = serde_json::to_string(&Index {
        tracks: tracks.to_vec(),
    })?;
    fs::write(index_file, json)
        .with_context(|| format!("writing index to {}", index_file.display()))
}
pub fn build(root: PathBuf, index_file: PathBuf, force: bool, tx: Sender<IndexMsg>) {
    let result = (|| -> Result<Scan> {
        let old = if force {
            HashMap::new()
        } else {
            load_existing(&index_file)
        };
        let mut unreadable: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let walker = WalkDir::new(&root)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name()
                    .to_str()
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(false)
            });
        for entry in walker {
            match entry {
                Ok(e) => {
                    if e.file_type().is_file() && is_audio(e.path()) {
                        files.push(e.into_path());
                    }
                }
                Err(err) => {
                    if let Some(p) = err.path() {
                        unreadable.push(p.to_path_buf());
                    }
                }
            }
        }
        let total = files.len();
        let _ = tx.send(IndexMsg::Progress { done: 0, total });
        let mut tracks = Vec::with_capacity(total);
        for (i, path) in files.into_iter().enumerate() {
            let (mtime, size) = fs::metadata(&path)
                .map(|m| {
                    let mt = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    (mt, m.len())
                })
                .unwrap_or((0, 0));
            let track = match old.get(&path) {
                Some(t) if t.mtime == mtime && t.size == size => t.clone(),
                _ => read_track(&path, mtime, size),
            };
            tracks.push(track);
            if i % 25 == 0 || i + 1 == total {
                let _ = tx.send(IndexMsg::Progress { done: i + 1, total });
            }
        }
        tracks.sort_by_cached_key(|t| t.sort_key());
        save(&index_file, &tracks)?;
        Ok(Scan { tracks, unreadable })
    })();
    let _ = tx.send(IndexMsg::Done(result));
}
