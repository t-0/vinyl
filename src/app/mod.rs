mod album;
mod artist;
mod eq_row;
mod fx_row;
mod normalize_state;
mod section;
mod settings;
mod sort_key;
#[cfg(test)]
mod tests;
mod view_state;
use crate::audio::Audio;
use crate::config::{Config, EditorMode, display_path};
use crate::fx::{
    EQ_RANGE_DB, EqMode, FxMode, FxParams, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX,
};
use crate::index::{self, IndexMsg, Track};
use crate::spectrum::Analyser;
pub use album::Album;
use anyhow::Result;
pub use artist::Artist;
pub use eq_row::EqRow;
pub use fx_row::FxRow;
pub use normalize_state::NormalizeState;
pub use section::Section;
use settings::write_settings;
pub use settings::{Settings, load_fx, load_settings};
pub use sort_key::SortKey;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use view_state::ViewState;
fn reveal_command(path: &std::path::Path) -> Command {
    let mut c = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    // The child would otherwise inherit our terminal and could print over the UI.
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}
/// Builds the editor invocation: `words` is the configured command split
/// into program and arguments. Every `{file}` in the arguments is replaced
/// by the track path; without one the path goes last. On macOS a `.app`
/// bundle is started through `open -a`, which is how bundles are launched;
/// `open` returns as soon as the app has the file, so that only makes sense
/// in GUI mode.
const FILE_PLACEHOLDER: &str = "{file}";
fn editor_command(words: &[String], path: &std::path::Path) -> Command {
    let program = words[0].trim_end_matches('/');
    let file = path.to_string_lossy();
    let mut args: Vec<String> = words[1..]
        .iter()
        .map(|w| w.replace(FILE_PLACEHOLDER, &file))
        .collect();
    if !words[1..].iter().any(|w| w.contains(FILE_PLACEHOLDER)) {
        args.push(file.into_owned());
    }
    let mut c = if cfg!(target_os = "macos") && program.ends_with(".app") {
        let mut c = Command::new("open");
        c.arg("-a").arg(program).args(&args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(&args);
        c
    };
    if let Some(dir) = path.parent() {
        c.current_dir(dir);
    }
    c
}
pub struct App {
    pub config: Config,
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub section: Section,
    pub view_songs: Vec<usize>,
    pub view_albums: Vec<usize>,
    pub view_artists: Vec<usize>,
    pub cursor_song: usize,
    pub cursor_album: usize,
    pub cursor_artist: usize,
    pub sort: SortKey,
    pub sort_open: bool,
    pub sort_row: usize,
    sort_order: Vec<usize>,
    song_pos: Vec<usize>,
    pub search: String,
    pub search_input: Option<String>,
    pub playing: Option<usize>,
    pending_play: Option<usize>,
    /// Load the pending track paused rather than playing (used when the
    /// last session's track is restored at startup).
    start_paused: bool,
    /// The track that was playing when the app last ran, selected and loaded
    /// paused once the index has been read; `None` after that.
    resume_track: Option<PathBuf>,
    pub offsets: [usize; 3],
    /// Per section: the cursor was moved by a track change rather than by the
    /// user, so if it landed off screen the list should scroll to centre it.
    pub recenter: [bool; 3],
    pub speed_pct: i32,
    pub shuffle: bool,
    shuffle_rng: u64,
    shuffle_history: Vec<usize>,
    pub audio: Option<Audio>,
    pub audio_error: Option<String>,
    /// A background level measurement for auto-normalisation: which track,
    /// and the channel its result arrives on.
    measuring: Option<(usize, Receiver<Result<f32>>)>,
    pub analyser: Analyser,
    pub fx: FxParams,
    pub fx_open: bool,
    pub fx_row: usize,
    pub eq_open: bool,
    pub eq_row: usize,
    pub status: String,
    pub status_at: Instant,
    pub warning: Option<String>,
    pub indexing: Option<(usize, usize)>,
    pub page_size: usize,
    index_rx: Option<Receiver<IndexMsg>>,
    /// A terminal audio editor waiting to be run by the main loop, which has
    /// to hand the screen over first (see `main::run_editor`).
    pub pending_editor: Option<(String, Command)>,
    pub should_quit: bool,
}
impl App {
    pub fn new(config: Config) -> Self {
        let settings = load_settings(&config);
        let (audio, audio_error) = match Audio::new(settings.fx) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let mut app = Self::build(config, settings.fx, audio, audio_error);
        app.shuffle = settings.shuffle;
        app.set_speed_pct(settings.speed_pct);
        app.resume_track = settings.last_track;
        app.start_index(false);
        app
    }
    fn build(
        config: Config,
        fx: FxParams,
        audio: Option<Audio>,
        audio_error: Option<String>,
    ) -> Self {
        Self {
            config,
            tracks: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            section: Section::Songs,
            view_songs: Vec::new(),
            view_albums: Vec::new(),
            view_artists: Vec::new(),
            sort: SortKey::Artist,
            sort_open: false,
            sort_row: 0,
            sort_order: Vec::new(),
            song_pos: Vec::new(),
            search: String::new(),
            search_input: None,
            cursor_song: 0,
            cursor_album: 0,
            cursor_artist: 0,
            playing: None,
            pending_play: None,
            start_paused: false,
            resume_track: None,
            offsets: [0; 3],
            recenter: [false; 3],
            speed_pct: 0,
            shuffle: false,
            shuffle_rng: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1,
            shuffle_history: Vec::new(),
            audio,
            audio_error,
            measuring: None,
            analyser: Analyser::new(),
            fx,
            fx_open: false,
            fx_row: 0,
            eq_open: false,
            eq_row: 0,
            status: String::new(),
            warning: None,
            status_at: Instant::now(),
            indexing: None,
            page_size: 10,
            index_rx: None,
            pending_editor: None,
            should_quit: false,
        }
    }
    pub fn start_index(&mut self, force: bool) {
        let (tx, rx) = mpsc::channel();
        let root = self.config.music_home.clone();
        let file = self.config.index_file.clone();
        thread::spawn(move || index::build(root, file, force, tx));
        self.index_rx = Some(rx);
        self.indexing = Some((0, 0));
    }
    pub fn poll_index(&mut self) {
        let Some(rx) = &self.index_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                IndexMsg::Progress { done, total } => self.indexing = Some((done, total)),
                IndexMsg::Done(r) => finished = Some(r),
            }
        }
        if let Some(result) = finished {
            self.index_rx = None;
            self.indexing = None;
            match result {
                Ok(scan) => {
                    let n = scan.tracks.len();
                    self.set_tracks(scan.tracks);
                    self.set_status(format!(
                        "indexed {n} tracks, {} albums, {} artists",
                        self.albums.len(),
                        self.artists.len()
                    ));
                    self.warning = if scan.unreadable.is_empty() {
                        None
                    } else {
                        let names: Vec<String> = scan
                            .unreadable
                            .iter()
                            .take(4)
                            .map(|p| display_path(p))
                            .collect();
                        let more = if scan.unreadable.len() > 4 {
                            format!(" and {} more", scan.unreadable.len() - 4)
                        } else {
                            String::new()
                        };
                        let hint = if cfg!(target_os = "macos") {
                            "Grant your terminal app access under System Settings → Privacy & Security → Media & Apple Music (and Files and Folders), then press i."
                        } else {
                            "Check the folder permissions, then press i to re-index."
                        };
                        Some(format!(
                            "{} folder{} could not be read: {}{}. {hint}",
                            scan.unreadable.len(),
                            if scan.unreadable.len() == 1 { "" } else { "s" },
                            names.join(", "),
                            more
                        ))
                    };
                }
                Err(e) => self.set_status(format!("index failed: {e:#}")),
            }
        }
    }
    fn set_tracks(&mut self, tracks: Vec<Track>) {
        let playing_path = self
            .playing
            .and_then(|i| self.tracks.get(i))
            .map(|t| t.path.clone());
        // A re-index (after the editor, or `i`) should not move the user's
        // selection or scroll: remember what each cursor is on.
        let keep = (!self.tracks.is_empty()).then(|| ViewState {
            song: self
                .view_songs
                .get(self.cursor_song)
                .map(|&i| self.tracks[i].path.clone()),
            album: self
                .view_albums
                .get(self.cursor_album)
                .map(|&a| (self.albums[a].artist.clone(), self.albums[a].name.clone())),
            artist: self
                .view_artists
                .get(self.cursor_artist)
                .map(|&r| self.artists[r].name.clone()),
            offsets: self.offsets,
        });
        // Keep levels measured since the index file was last read, as long
        // as the file itself has not changed.
        let measured: std::collections::HashMap<&std::path::Path, (u64, u64, f32)> = self
            .tracks
            .iter()
            .filter_map(|t| t.gain_db.map(|g| (t.path.as_path(), (t.mtime, t.size, g))))
            .collect();
        let mut tracks = tracks;
        for t in tracks.iter_mut().filter(|t| t.gain_db.is_none()) {
            if let Some(&(mtime, size, g)) = measured.get(t.path.as_path())
                && t.mtime == mtime
                && t.size == size
            {
                t.gain_db = Some(g);
            }
        }
        self.tracks = tracks;
        self.albums.clear();
        self.artists.clear();
        let mut i = 0;
        while i < self.tracks.len() {
            let t = &self.tracks[i];
            let artist = t.group_artist().to_string();
            let name = t.album.clone();
            let mut j = i + 1;
            while j < self.tracks.len()
                && self.tracks[j].group_artist().eq_ignore_ascii_case(&artist)
                && self.tracks[j].album.eq_ignore_ascii_case(&name)
            {
                j += 1;
            }
            let year = self.tracks[i..j].iter().filter_map(|t| t.year).min();
            self.albums.push(Album {
                artist,
                name,
                year,
                start: i,
                end: j,
            });
            i = j;
        }
        let mut a = 0;
        while a < self.albums.len() {
            let name = self.albums[a].artist.clone();
            let mut b = a + 1;
            while b < self.albums.len() && self.albums[b].artist.eq_ignore_ascii_case(&name) {
                b += 1;
            }
            self.artists.push(Artist {
                name,
                album_start: a,
                album_end: b,
                track_start: self.albums[a].start,
                track_end: self.albums[b - 1].end,
            });
            a = b;
        }
        self.playing = playing_path.and_then(|p| self.tracks.iter().position(|t| t.path == p));
        self.rebuild_sort_order();
        self.apply_search();
        if let Some(keep) = keep {
            self.restore_view(keep);
        }
        // First index after startup: pick up where the last session left
        // off, with the track cued but not playing.
        if let Some(p) = self.resume_track.take()
            && self.playing.is_none()
            && let Some(i) = self.tracks.iter().position(|t| t.path == p)
        {
            self.start_paused = true;
            self.play_index(i);
        }
    }
    fn restore_view(&mut self, keep: ViewState) {
        if let Some(p) = keep.song
            && let Some(i) = self.tracks.iter().position(|t| t.path == p)
            && let Some(pos) = self.song_view_pos(i)
        {
            self.cursor_song = pos;
        }
        if let Some((artist, name)) = keep.album
            && let Some(a) = self
                .albums
                .iter()
                .position(|al| al.artist == artist && al.name == name)
            && self.view_albums.binary_search(&a).is_ok()
        {
            self.cursor_album = Self::view_pos(&self.view_albums, a);
        }
        if let Some(name) = keep.artist
            && let Some(r) = self.artists.iter().position(|ar| ar.name == name)
            && self.view_artists.binary_search(&r).is_ok()
        {
            self.cursor_artist = Self::view_pos(&self.view_artists, r);
        }
        // The lists are drawn with these offsets clamped so the cursor stays
        // visible, so a stale offset is harmless.
        self.offsets = keep.offsets;
        self.recenter = [false; 3];
    }
    fn rebuild_sort_order(&mut self) {
        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        match self.sort {
            SortKey::Artist => {}
            SortKey::Track => {
                let tracks = &self.tracks;
                order.sort_by_cached_key(|&i| {
                    (
                        tracks[i].title.to_lowercase(),
                        tracks[i].artist.to_lowercase(),
                        i,
                    )
                });
            }
            SortKey::Album => {
                let tracks = &self.tracks;
                order.sort_by_cached_key(|&i| {
                    (
                        tracks[i].album.to_lowercase(),
                        tracks[i].group_artist().to_lowercase(),
                        i,
                    )
                });
            }
            SortKey::ArtistTrack => {
                let tracks = &self.tracks;
                order.sort_by_cached_key(|&i| {
                    (
                        tracks[i].artist.to_lowercase(),
                        tracks[i].title.to_lowercase(),
                        tracks[i].album.to_lowercase(),
                        i,
                    )
                });
            }
        }
        self.sort_order = order;
    }
    pub fn open_sort(&mut self) {
        self.sort_row = SortKey::ALL
            .iter()
            .position(|&k| k == self.sort)
            .unwrap_or(0);
        self.sort_open = true;
    }
    pub fn sort_move(&mut self, delta: i64) {
        let n = SortKey::ALL.len() as i64;
        self.sort_row = (self.sort_row as i64 + delta).rem_euclid(n) as usize;
    }
    pub fn apply_sort(&mut self) {
        self.sort_open = false;
        let key = SortKey::ALL[self.sort_row];
        if key != self.sort {
            self.sort = key;
            self.rebuild_sort_order();
            self.apply_search();
            self.offsets[0] = 0;
        }
        self.set_status(format!("sorted by {}", key.label().to_lowercase()));
    }
    pub fn apply_search(&mut self) {
        let q = self.search.trim().to_lowercase();
        let matched: Vec<bool> = if q.is_empty() {
            vec![true; self.tracks.len()]
        } else {
            self.tracks
                .iter()
                .map(|t| {
                    t.artist.to_lowercase().contains(&q)
                        || t.album_artist
                            .as_deref()
                            .map(|a| a.to_lowercase().contains(&q))
                            .unwrap_or(false)
                        || t.album.to_lowercase().contains(&q)
                        || t.title.to_lowercase().contains(&q)
                })
                .collect()
        };
        self.view_songs = self
            .sort_order
            .iter()
            .copied()
            .filter(|&i| matched[i])
            .collect();
        self.song_pos = vec![usize::MAX; self.tracks.len()];
        for (pos, &i) in self.view_songs.iter().enumerate() {
            self.song_pos[i] = pos;
        }
        self.view_albums = (0..self.albums.len())
            .filter(|&a| {
                matched[self.albums[a].start..self.albums[a].end]
                    .iter()
                    .any(|&m| m)
            })
            .collect();
        let album_matched: Vec<bool> = {
            let mut v = vec![false; self.albums.len()];
            for &a in &self.view_albums {
                v[a] = true;
            }
            v
        };
        self.view_artists = (0..self.artists.len())
            .filter(|&r| {
                album_matched[self.artists[r].album_start..self.artists[r].album_end]
                    .iter()
                    .any(|&m| m)
            })
            .collect();
        self.cursor_song = 0;
        self.cursor_album = 0;
        self.cursor_artist = 0;
        self.offsets = [0; 3];
        if let Some(i) = self.playing
            && self.song_view_pos(i).is_some()
        {
            self.sync_cursors_to(i);
        }
    }
    pub fn open_search(&mut self) {
        self.search_input = Some(String::new());
    }
    pub fn cancel_search(&mut self) {
        self.search_input = None;
    }
    pub fn submit_search(&mut self) {
        let Some(q) = self.search_input.take() else {
            return;
        };
        self.search = q.trim().to_string();
        self.apply_search();
        self.section = Section::Songs;
        if self.search.is_empty() {
            self.set_status("showing everything");
        } else {
            let n = self.view_songs.len();
            self.set_status(format!(
                "{n} song{} match \"{}\"",
                if n == 1 { "" } else { "s" },
                self.search
            ));
        }
    }
    fn first_in_view(&self, start: usize, end: usize) -> Option<usize> {
        (start..end.min(self.song_pos.len())).find(|&i| self.song_pos[i] != usize::MAX)
    }
    fn song_view_pos(&self, idx: usize) -> Option<usize> {
        self.song_pos.get(idx).copied().filter(|&p| p != usize::MAX)
    }
    fn view_pos(view: &[usize], idx: usize) -> usize {
        match view.binary_search(&idx) {
            Ok(p) => p,
            Err(p) => p.min(view.len().saturating_sub(1)),
        }
    }
    fn cursor_track(&self) -> Option<usize> {
        match self.section {
            Section::Songs => self.view_songs.get(self.cursor_song).copied(),
            Section::Albums => self
                .view_albums
                .get(self.cursor_album)
                .and_then(|&a| self.first_in_view(self.albums[a].start, self.albums[a].end)),
            Section::Artists => self.view_artists.get(self.cursor_artist).and_then(|&r| {
                self.first_in_view(self.artists[r].track_start, self.artists[r].track_end)
            }),
        }
    }
    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_at = Instant::now();
    }
    pub fn album_of(&self, track_idx: usize) -> Option<usize> {
        self.albums
            .iter()
            .position(|a| track_idx >= a.start && track_idx < a.end)
    }
    pub fn artist_of_album(&self, album_idx: usize) -> Option<usize> {
        self.artists
            .iter()
            .position(|a| album_idx >= a.album_start && album_idx < a.album_end)
    }
    fn sync_cursors_to(&mut self, track_idx: usize) {
        self.recenter = [true; 3];
        if let Some(p) = self.song_view_pos(track_idx) {
            self.cursor_song = p;
        } else {
            self.cursor_song = self
                .cursor_song
                .min(self.view_songs.len().saturating_sub(1));
        }
        if let Some(al) = self.album_of(track_idx) {
            self.cursor_album = Self::view_pos(&self.view_albums, al);
            if let Some(ar) = self.artist_of_album(al) {
                self.cursor_artist = Self::view_pos(&self.view_artists, ar);
            }
        }
    }
    pub fn speed(&self) -> f32 {
        1.0 + self.speed_pct as f32 / 100.0
    }
    pub fn playing_track(&self) -> Option<&Track> {
        self.playing.and_then(|i| self.tracks.get(i))
    }
    fn anchor_pos(&self) -> Option<usize> {
        if self.view_songs.is_empty() {
            return None;
        }
        if let Some(pos) = self.playing.and_then(|p| self.song_view_pos(p)) {
            return Some(pos);
        }
        let idx = self.cursor_track().unwrap_or(self.view_songs[0]);
        Some(self.song_view_pos(idx).unwrap_or(0))
    }
    pub fn play_index(&mut self, i: usize) {
        if i >= self.tracks.len() {
            return;
        }
        self.playing = Some(i);
        self.pending_play = Some(i);
        self.sync_cursors_to(i);
        self.save_settings();
    }
    pub fn flush_play(&mut self) {
        let Some(i) = self.pending_play.take() else {
            return;
        };
        let paused = std::mem::take(&mut self.start_paused);
        let Some(track) = self.tracks.get(i) else {
            return;
        };
        let path = track.path.clone();
        let speed = self.speed();
        // The FX chain reads its gain when the source is built, so the
        // track's normalisation gain has to be in place before `load`.
        self.sync_track_gain();
        match self.audio.as_mut() {
            Some(audio) => match audio.load(&path, speed) {
                Ok(()) => {
                    if paused {
                        audio.pause();
                    }
                    self.status.clear();
                }
                Err(e) => {
                    self.playing = None;
                    let mut msg = format!("cannot play: {e:#}");
                    if cfg!(target_os = "macos")
                        && e.chain().any(|c| {
                            c.downcast_ref::<std::io::Error>()
                                .map(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
                                .unwrap_or(false)
                        })
                    {
                        msg.push_str(" — macOS is blocking this terminal: System Settings → Privacy & Security → Media & Apple Music (and Files and Folders)");
                    }
                    self.set_status(msg);
                }
            },
            None => self.set_status(format!(
                "no audio output: {}",
                self.audio_error.as_deref().unwrap_or("unknown error")
            )),
        }
    }
    fn play_cursor(&mut self) {
        if let Some(i) = self.cursor_track() {
            self.play_index(i);
        }
    }
    pub fn toggle_play(&mut self) {
        if self.playing.is_some() {
            if let Some(a) = &self.audio {
                a.toggle_pause();
            }
        } else {
            self.play_cursor();
        }
    }
    pub fn step_track(&mut self, delta: i64) {
        if self.shuffle && self.view_songs.len() > 1 {
            self.step_shuffle(delta);
            return;
        }
        let Some(pos) = self.anchor_pos() else { return };
        let n = self.view_songs.len() as i64;
        let next = (pos as i64 + delta).rem_euclid(n) as usize;
        self.play_index(self.view_songs[next]);
    }
    fn next_random(&mut self) -> u64 {
        let mut x = self.shuffle_rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.shuffle_rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn step_shuffle(&mut self, delta: i64) {
        if delta < 0 {
            while let Some(prev) = self.shuffle_history.pop() {
                if Some(prev) != self.playing && self.tracks.get(prev).is_some() {
                    self.play_index(prev);
                    return;
                }
            }
            return;
        }
        let n = self.view_songs.len();
        let r = (self.next_random() % n as u64) as usize;
        let mut pick = self.view_songs[r];
        if Some(pick) == self.playing {
            let pos = self.song_view_pos(pick).unwrap_or(0);
            pick = self.view_songs[(pos + 1) % n];
        }
        if let Some(cur) = self.playing {
            self.shuffle_history.push(cur);
            if self.shuffle_history.len() > 500 {
                self.shuffle_history.remove(0);
            }
        }
        self.play_index(pick);
    }
    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.shuffle_history.clear();
        self.set_status(if self.shuffle {
            "Shuffle on"
        } else {
            "Shuffle off"
        });
        self.save_settings();
    }
    pub fn move_cursor(&mut self, delta: i64) {
        let (cur, len) = match self.section {
            Section::Songs => (&mut self.cursor_song, self.view_songs.len()),
            Section::Albums => (&mut self.cursor_album, self.view_albums.len()),
            Section::Artists => (&mut self.cursor_artist, self.view_artists.len()),
        };
        if len == 0 {
            return;
        }
        *cur = (*cur as i64).saturating_add(delta).clamp(0, len as i64 - 1) as usize;
        self.recenter[self.section as usize] = false;
    }
    pub fn activate_cursor(&mut self) {
        self.play_cursor();
    }
    pub fn seek(&mut self, delta_secs: f64) {
        let Some(t) = self.playing_track() else {
            return;
        };
        let total = t.duration_secs;
        if let Some(a) = &self.audio {
            a.seek_by(delta_secs, total);
        }
    }
    pub fn adjust_speed(&mut self, delta: i32) {
        self.set_speed_pct(self.speed_pct + delta);
        self.save_settings();
    }
    pub fn reset_speed(&mut self) {
        self.set_speed_pct(0);
        self.save_settings();
    }
    fn set_speed_pct(&mut self, pct: i32) {
        self.speed_pct = pct.clamp(-50, 100);
        let s = self.speed();
        if let Some(a) = &self.audio {
            a.set_speed(s);
        }
    }
    pub fn reveal_in_finder(&mut self) {
        let idx = self.playing.or_else(|| self.cursor_track());
        let Some(t) = idx.and_then(|i| self.tracks.get(i)) else {
            return;
        };
        let path = t.path.clone();
        match reveal_command(&path).spawn() {
            Ok(_) => self.set_status(format!("revealed {}", path.display())),
            Err(e) => self.set_status(format!("could not open file manager: {e}")),
        }
    }
    /// `E`: opens the selected track in the configured audio editor. A GUI
    /// editor is started and left alone; a CLI one is queued for the main
    /// loop, which gives it the terminal and waits.
    pub fn edit_in_editor(&mut self) {
        let Some(words) = self.config.audio_editor.clone() else {
            self.set_status("no audio editor: set audio.editor in ~/.vinyl");
            return;
        };
        let idx = self.cursor_track().or(self.playing);
        let Some(t) = idx.and_then(|i| self.tracks.get(i)) else {
            return;
        };
        let path = t.path.clone();
        let name = std::path::Path::new(words[0].trim_end_matches('/'))
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| words[0].clone());
        let mut cmd = editor_command(&words, &path);
        match self.config.audio_editor_mode {
            EditorMode::Gui => {
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                match cmd.spawn() {
                    Ok(_) => self.set_status(format!(
                        "opened {} in {name}",
                        self.config.library_path(&path)
                    )),
                    Err(e) => self.set_status(format!("could not start {name}: {e}")),
                }
            }
            EditorMode::Cli => {
                cmd.stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                self.pending_editor = Some((name, cmd));
            }
        }
    }
    /// Called by the main loop once a CLI editor has exited and the UI is
    /// back on screen.
    pub fn editor_finished(
        &mut self,
        program: &str,
        result: std::io::Result<std::process::ExitStatus>,
    ) {
        match result {
            Ok(st) if st.success() => {
                self.set_status(format!("{program} finished"));
                // Tags or length may have changed: pick that up without
                // rereading everything.
                if self.index_rx.is_none() {
                    self.start_index(false);
                }
            }
            Ok(st) => self.set_status(format!("{program} exited with {st}")),
            Err(e) => self.set_status(format!("could not start {program}: {e}")),
        }
    }
    pub fn fx_rows(&self) -> Vec<FxRow> {
        let mut rows = vec![FxRow::Mode];
        match self.fx.mode {
            FxMode::Off => {}
            FxMode::Vinyl => rows.extend([FxRow::Scratch, FxRow::Distortion]),
            FxMode::ReelToReel | FxMode::Cassette => rows.extend([
                FxRow::Age,
                FxRow::Hiss,
                FxRow::Saturation,
                FxRow::Wow,
                FxRow::Flutter,
            ]),
        }
        rows
    }
    pub fn fx_value(&self, row: FxRow) -> u8 {
        match row {
            FxRow::Mode => 0,
            FxRow::Scratch => self.fx.scratch,
            FxRow::Distortion => self.fx.distortion,
            FxRow::Age => self.fx.age,
            FxRow::Saturation => self.fx.saturation,
            FxRow::Wow => self.fx.wow,
            FxRow::Flutter => self.fx.flutter,
            FxRow::Hiss => self.fx.hiss,
        }
    }
    pub fn fx_move(&mut self, delta: i64) {
        let n = self.fx_rows().len() as i64;
        self.fx_row = (self.fx_row as i64 + delta).rem_euclid(n) as usize;
    }
    pub fn fx_adjust(&mut self, delta: i32) {
        let rows = self.fx_rows();
        let Some(&row) = rows.get(self.fx_row) else {
            return;
        };
        let bump = |v: &mut u8| *v = (*v as i32 + delta).clamp(0, 100) as u8;
        match row {
            FxRow::Mode => {
                let i = FxMode::ALL
                    .iter()
                    .position(|&m| m == self.fx.mode)
                    .unwrap_or(0) as i64;
                let step = if delta < 0 { -1 } else { 1 };
                self.fx.mode =
                    FxMode::ALL[(i + step).rem_euclid(FxMode::ALL.len() as i64) as usize];
                self.fx_row = 0;
            }
            FxRow::Scratch => bump(&mut self.fx.scratch),
            FxRow::Distortion => bump(&mut self.fx.distortion),
            FxRow::Age => bump(&mut self.fx.age),
            FxRow::Saturation => bump(&mut self.fx.saturation),
            FxRow::Wow => bump(&mut self.fx.wow),
            FxRow::Flutter => bump(&mut self.fx.flutter),
            FxRow::Hiss => bump(&mut self.fx.hiss),
        }
        self.push_fx();
    }
    pub fn fx_reset_row(&mut self) {
        let rows = self.fx_rows();
        let Some(&row) = rows.get(self.fx_row) else {
            return;
        };
        let def = FxParams::default();
        match row {
            FxRow::Mode => self.fx.mode = FxMode::Off,
            FxRow::Scratch => self.fx.scratch = def.scratch,
            FxRow::Distortion => self.fx.distortion = def.distortion,
            FxRow::Age => self.fx.age = def.age,
            FxRow::Saturation => self.fx.saturation = def.saturation,
            FxRow::Wow => self.fx.wow = def.wow,
            FxRow::Flutter => self.fx.flutter = def.flutter,
            FxRow::Hiss => self.fx.hiss = def.hiss,
        }
        self.push_fx();
    }
    fn push_fx(&mut self) {
        if let Some(a) = &self.audio {
            a.set_fx(self.fx);
        }
    }
    pub fn close_fx(&mut self) {
        self.fx_open = false;
        self.save_settings();
    }
    fn settings(&self) -> Settings {
        // Before the index has loaded nothing is playing yet; keep the track
        // waiting to be restored rather than forgetting it.
        let last_track = self
            .playing
            .and_then(|i| self.tracks.get(i))
            .map(|t| t.path.clone())
            .or_else(|| self.resume_track.clone());
        Settings {
            fx: self.fx,
            shuffle: self.shuffle,
            speed_pct: self.speed_pct,
            last_track,
        }
    }
    fn save_settings(&mut self) {
        if let Err(e) = write_settings(&self.config, &self.settings()) {
            self.set_status(format!("could not save settings: {e:#}"));
        }
    }
    pub fn eq_rows(&self) -> Vec<EqRow> {
        let mut rows = vec![EqRow::Preamp, EqRow::Normalize, EqRow::Mode];
        rows.extend((0..self.fx.eq.mode.bands()).map(EqRow::Band));
        rows.push(EqRow::Loudness);
        rows.push(EqRow::Stereo);
        if self.fx.stereo == StereoMode::Width {
            rows.push(EqRow::Width);
        }
        rows
    }
    pub fn eq_move(&mut self, delta: i64) {
        let n = self.eq_rows().len() as i64;
        self.eq_row = (self.eq_row as i64 + delta).rem_euclid(n) as usize;
    }
    pub fn eq_adjust(&mut self, delta: i32) {
        let rows = self.eq_rows();
        let Some(&row) = rows.get(self.eq_row) else {
            return;
        };
        let step = if delta < 0 { -1 } else { 1 };
        match row {
            EqRow::Preamp => {
                self.fx.preamp = (self.fx.preamp as i32 + delta)
                    .clamp(PREAMP_MIN_DB as i32, PREAMP_MAX_DB as i32)
                    as i8
            }
            EqRow::Normalize => self.set_normalize(!self.fx.auto_normalize),
            EqRow::Mode => {
                let i = EqMode::ALL
                    .iter()
                    .position(|&m| m == self.fx.eq.mode)
                    .unwrap_or(0) as i64;
                self.fx.eq.mode =
                    EqMode::ALL[(i + step).rem_euclid(EqMode::ALL.len() as i64) as usize];
            }
            EqRow::Band(b) => {
                if let Some(g) = self.fx.eq.gain_mut(b) {
                    *g = (*g as i32 + delta).clamp(-(EQ_RANGE_DB as i32), EQ_RANGE_DB as i32) as i8;
                }
            }
            EqRow::Loudness => self.fx.loudness = !self.fx.loudness,
            EqRow::Stereo => {
                let i = StereoMode::ALL
                    .iter()
                    .position(|&m| m == self.fx.stereo)
                    .unwrap_or(0) as i64;
                self.fx.stereo =
                    StereoMode::ALL[(i + step).rem_euclid(StereoMode::ALL.len() as i64) as usize];
            }
            EqRow::Width => {
                self.fx.width = (self.fx.width as i32 + delta * 5).clamp(0, WIDTH_MAX as i32) as u8
            }
        }
        self.eq_row = self.eq_row.min(self.eq_rows().len() - 1);
        self.push_fx();
    }
    pub fn eq_reset_row(&mut self) {
        let rows = self.eq_rows();
        let Some(&row) = rows.get(self.eq_row) else {
            return;
        };
        match row {
            EqRow::Preamp => self.fx.preamp = 0,
            EqRow::Normalize => self.set_normalize(false),
            EqRow::Mode => self.fx.eq.mode = EqMode::Off,
            EqRow::Band(b) => {
                if let Some(g) = self.fx.eq.gain_mut(b) {
                    *g = 0;
                }
            }
            EqRow::Loudness => self.fx.loudness = false,
            EqRow::Stereo => self.fx.stereo = StereoMode::Normal,
            EqRow::Width => self.fx.width = 100,
        }
        self.eq_row = self.eq_row.min(self.eq_rows().len() - 1);
        self.push_fx();
    }
    fn set_normalize(&mut self, on: bool) {
        self.fx.auto_normalize = on;
        self.sync_track_gain();
    }
    /// What the Preamp row should say about normalisation: the gain in use,
    /// or that it is still being measured.
    pub fn normalize_state(&self) -> NormalizeState {
        if !self.fx.auto_normalize {
            return NormalizeState::Off;
        }
        match self.playing.and_then(|i| self.tracks.get(i)) {
            Some(t) => match t.gain_db {
                Some(g) => NormalizeState::Gain(g),
                None => NormalizeState::Measuring,
            },
            None => NormalizeState::Idle,
        }
    }
    pub fn close_eq(&mut self) {
        self.eq_open = false;
        self.save_settings();
    }
    /// Points the FX chain at the playing track's normalisation gain (0 dB
    /// while unknown) and starts measuring it if needed.
    fn sync_track_gain(&mut self) {
        let playing = self
            .playing
            .and_then(|i| self.tracks.get(i).map(|t| (i, t.gain_db)));
        self.fx.track_gain = match playing {
            Some((_, Some(g))) if self.fx.auto_normalize => g,
            _ => 0.0,
        };
        self.push_fx();
        if let Some((i, None)) = playing
            && self.fx.auto_normalize
            && self.measuring.as_ref().map(|(m, _)| *m) != Some(i)
        {
            let path = self.tracks[i].path.clone();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(crate::audio::measure_gain(&path));
            });
            self.measuring = Some((i, rx));
        }
    }
    pub fn poll_measurement(&mut self) {
        let Some((i, rx)) = &self.measuring else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        let i = *i;
        self.measuring = None;
        match result {
            Ok(g) => {
                if let Some(t) = self.tracks.get_mut(i) {
                    t.gain_db = Some(g);
                }
                if self.playing == Some(i) {
                    self.sync_track_gain();
                }
                // Remember it for next time, off the UI thread.
                let file = self.config.index_file.clone();
                let tracks = self.tracks.clone();
                thread::spawn(move || {
                    let _ = index::save(&file, &tracks);
                });
            }
            Err(e) => self.set_status(format!("could not measure level: {e:#}")),
        }
    }
    pub fn toggle_loudness(&mut self) {
        self.fx.loudness = !self.fx.loudness;
        self.push_fx();
        self.set_status(if self.fx.loudness {
            "Loudness on"
        } else {
            "Loudness off"
        });
        self.save_settings();
    }
    pub fn audio_active(&self) -> bool {
        self.playing.is_some()
            && self
                .audio
                .as_ref()
                .map(|a| !a.is_paused() && !a.finished())
                .unwrap_or(false)
    }
    pub fn update_spectrum(&mut self) {
        let active = self.audio_active();
        let tap = self.audio.as_ref().map(|a| a.tap().clone());
        self.analyser.update(tap.as_ref(), active);
    }
    /// Scroll offset for a list of `len` rows shown `height` at a time.
    ///
    /// Normally the list scrolls just enough to keep the cursor visible. When
    /// `recenter` is set (the cursor was moved by a track change, e.g. shuffle
    /// jumping somewhere far away) and the cursor is off screen, the list is
    /// scrolled so the cursor sits in the middle instead of at an edge.
    pub fn scroll_offset(
        offset: usize,
        cursor: usize,
        len: usize,
        height: usize,
        recenter: bool,
    ) -> usize {
        let height = height.max(1);
        let mut offset = offset.min(len.saturating_sub(1));
        let visible = cursor >= offset && cursor < offset + height;
        if recenter && !visible {
            offset = cursor
                .saturating_sub(height / 2)
                .min(len.saturating_sub(height));
        }
        if cursor < offset {
            offset = cursor;
        } else if cursor >= offset + height {
            offset = cursor + 1 - height;
        }
        offset
    }
    pub fn tick(&mut self) -> Result<()> {
        self.poll_index();
        self.poll_measurement();
        if self.pending_play.is_none()
            && let (Some(_), Some(audio)) = (self.playing, &self.audio)
            && audio.finished()
            && !audio.is_paused()
        {
            self.step_track(1);
        }
        Ok(())
    }
}
