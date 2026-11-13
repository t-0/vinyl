use crate::audio::Audio;
use crate::config::{Config, EditorMode, display_path};
use crate::fx::{
    EQ_RANGE_DB, EqMode, FxParams, Mode, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX,
};
use crate::index::{self, IndexMsg, Track};
use crate::spectrum::Analyser;
use anyhow::Result;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Songs,
    Albums,
    Artists,
}
impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Songs => "Songs",
            Section::Albums => "Albums",
            Section::Artists => "Artists",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Album {
    pub artist: String,
    pub name: String,
    pub year: Option<u32>,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug)]
pub struct Artist {
    pub name: String,
    pub album_start: usize,
    pub album_end: usize,
    pub track_start: usize,
    pub track_end: usize,
}
/// What the three list cursors were on, by identity rather than index, so
/// they can be put back after the track list is rebuilt.
struct ViewState {
    song: Option<std::path::PathBuf>,
    album: Option<(String, String)>,
    artist: Option<String>,
    offsets: [usize; 3],
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
        let fx = load_fx(&config);
        let (audio, audio_error) = match Audio::new(fx) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let mut app = Self::build(config, fx, audio, audio_error);
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
    }
    pub fn flush_play(&mut self) {
        let Some(i) = self.pending_play.take() else {
            return;
        };
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
                Ok(()) => self.status.clear(),
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
        self.speed_pct = (self.speed_pct + delta).clamp(-50, 100);
        let s = self.speed();
        if let Some(a) = &self.audio {
            a.set_speed(s);
        }
    }
    pub fn reset_speed(&mut self) {
        self.speed_pct = 0;
        if let Some(a) = &self.audio {
            a.set_speed(1.0);
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
            Mode::Off => {}
            Mode::Vinyl => rows.extend([FxRow::Scratch, FxRow::Distortion]),
            Mode::ReelToReel | Mode::Cassette => rows.extend([
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
                let i = Mode::ALL
                    .iter()
                    .position(|&m| m == self.fx.mode)
                    .unwrap_or(0) as i64;
                let step = if delta < 0 { -1 } else { 1 };
                self.fx.mode = Mode::ALL[(i + step).rem_euclid(Mode::ALL.len() as i64) as usize];
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
            FxRow::Mode => self.fx.mode = Mode::Off,
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
    fn save_settings(&mut self) {
        if let Err(e) = save_fx(&self.config, &self.fx) {
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Artist,
    Track,
    Album,
    ArtistTrack,
}
impl SortKey {
    pub const ALL: [SortKey; 4] = [
        SortKey::Artist,
        SortKey::Track,
        SortKey::Album,
        SortKey::ArtistTrack,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Artist => "Artist",
            SortKey::Track => "Track",
            SortKey::Album => "Album",
            SortKey::ArtistTrack => "Artist/Track",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            SortKey::Artist => "artist, then album, then track number",
            SortKey::Track => "track title, then artist",
            SortKey::Album => "album, then artist, then track number",
            SortKey::ArtistTrack => "artist, then track title, then album",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NormalizeState {
    Off,
    /// On, but nothing is playing.
    Idle,
    Measuring,
    Gain(f32),
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EqRow {
    Preamp,
    Normalize,
    Mode,
    Band(usize),
    Loudness,
    Stereo,
    Width,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxRow {
    Mode,
    Scratch,
    Distortion,
    Age,
    Saturation,
    Wow,
    Flutter,
    Hiss,
}
impl FxRow {
    pub fn label(self) -> &'static str {
        match self {
            FxRow::Mode => "Mode",
            FxRow::Scratch => "Scratch",
            FxRow::Distortion => "Distortion",
            FxRow::Age => "Tape age",
            FxRow::Saturation => "Saturation",
            FxRow::Wow => "Wow",
            FxRow::Flutter => "Flutter",
            FxRow::Hiss => "Hiss",
        }
    }
}
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
fn fx_file(config: &Config) -> std::path::PathBuf {
    config.index_file.with_file_name(".vinyl-fx.json")
}
pub fn load_fx(config: &Config) -> FxParams {
    std::fs::read_to_string(fx_file(config))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
fn save_fx(config: &Config, fx: &FxParams) -> Result<()> {
    let json = serde_json::to_string_pretty(fx)?;
    std::fs::write(fx_file(config), json)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant as TInstant;
    fn temp_config(tag: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("vinyl-test-{}-{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        Config {
            music_home: dir.clone(),
            index_file: dir.join("index.json"),
            audio_editor: None,
            audio_editor_mode: EditorMode::Gui,
        }
    }
    fn track(artist: &str, album: &str, title: &str, n: u32) -> Track {
        Track {
            path: PathBuf::from(format!("/m/{artist}/{album}/{n:02} {title}.mp3")),
            mtime: 0,
            size: 1,
            artist: artist.into(),
            album_artist: None,
            album: album.into(),
            title: title.into(),
            year: None,
            track: Some(n),
            disc: None,
            duration_secs: 180.0,
            filetype: "mp3".into(),
            gain_db: None,
        }
    }
    fn args_of(c: &Command) -> Vec<String> {
        c.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }
    #[test]
    fn editor_file_goes_last_unless_placed() {
        let p = std::path::Path::new("/m/a b/x.mp3");
        let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        let c = editor_command(&w("ed --v"), p);
        assert_eq!(c.get_program(), "ed");
        assert_eq!(args_of(&c), ["--v", "/m/a b/x.mp3"]);
        let c = editor_command(&w("ed --in={file} --out={file}.bak last"), p);
        assert_eq!(
            args_of(&c),
            ["--in=/m/a b/x.mp3", "--out=/m/a b/x.mp3.bak", "last"]
        );
        let c = editor_command(&w("ed {file}"), p);
        assert_eq!(args_of(&c), ["/m/a b/x.mp3"]);
    }
    #[test]
    fn measured_gain_survives_reindex_and_feeds_preamp() {
        let mut app = app_with("gain", library());
        app.tracks[3].gain_db = Some(-4.5);
        let path = app.tracks[3].path.clone();
        app.set_tracks(library());
        let t = app.tracks.iter().find(|t| t.path == path).unwrap();
        assert_eq!(t.gain_db, Some(-4.5));
        let mut changed = library();
        changed.iter_mut().find(|t| t.path == path).unwrap().size = 2;
        app.set_tracks(changed);
        let t = app.tracks.iter().find(|t| t.path == path).unwrap();
        assert_eq!(t.gain_db, None, "a changed file needs a new measurement");
        // The gain only reaches the FX chain while normalisation is on and
        // that track is playing.
        app.tracks
            .iter_mut()
            .find(|t| t.path == path)
            .unwrap()
            .gain_db = Some(-4.5);
        let idx = app.tracks.iter().position(|t| t.path == path).unwrap();
        app.playing = Some(idx);
        app.sync_track_gain();
        assert_eq!(app.fx.track_gain, 0.0);
        assert!(matches!(app.normalize_state(), NormalizeState::Off));
        app.set_normalize(true);
        assert_eq!(app.fx.track_gain, -4.5);
        assert!(app.measuring.is_none());
        assert!(matches!(app.normalize_state(), NormalizeState::Gain(g) if g == -4.5));
        app.playing = Some((idx + 1) % app.tracks.len());
        app.sync_track_gain();
        assert_eq!(app.fx.track_gain, 0.0);
        assert!(
            app.measuring.is_some(),
            "an unmeasured track starts a measurement"
        );
        assert!(matches!(app.normalize_state(), NormalizeState::Measuring));
        app.set_normalize(false);
        assert_eq!(app.fx.track_gain, 0.0);
    }
    #[test]
    fn reindex_keeps_selection_and_scroll() {
        let mut app = app_with("keep", library());
        app.move_cursor(7);
        app.offsets = [3, 1, 0];
        let before = app.view_songs[app.cursor_song];
        let path = app.tracks[before].path.clone();
        app.section = Section::Albums;
        app.move_cursor(2);
        let album = app.albums[app.view_albums[app.cursor_album]].name.clone();
        // Rebuild from a copy with one extra track at the front of the order.
        let mut tracks = library();
        tracks.push(track("AAA First", "Early", "Opener", 1));
        tracks.sort_by_cached_key(|t| t.sort_key());
        app.set_tracks(tracks);
        assert_eq!(app.tracks[app.view_songs[app.cursor_song]].path, path);
        assert_eq!(app.albums[app.view_albums[app.cursor_album]].name, album);
        assert_eq!(app.offsets, [3, 1, 0]);
        assert_eq!(app.recenter, [false; 3]);
    }
    fn app_with(tag: &str, mut tracks: Vec<Track>) -> App {
        tracks.sort_by_cached_key(|t| t.sort_key());
        let mut app = App::build(temp_config(tag), FxParams::default(), None, None);
        app.set_tracks(tracks);
        app
    }
    fn library() -> Vec<Track> {
        let mut v = Vec::new();
        for (ar, al, titles) in [
            (
                "Aphex Twin",
                "Drukqs",
                ["Vordhosbn", "Avril 14th", "Omgyjya-Switch7"],
            ),
            (
                "Aphex Twin",
                "Syro",
                ["Minipops", "Xmas Evet10", "Produk 29"],
            ),
            (
                "Boards of Canada",
                "Geogaddi",
                ["Music Is Math", "Julie and Candy", "1969"],
            ),
            (
                "Sasha",
                "Unknown Album",
                ["Ally Pally", "Xpander", "Wavy Gravy"],
            ),
        ] {
            for (i, t) in titles.iter().enumerate() {
                v.push(track(ar, al, t, i as u32 + 1));
            }
        }
        v
    }
    fn invariants(app: &App) {
        let n = app.view_songs.len();
        assert!(
            n == 0 || app.cursor_song < n,
            "cursor {} >= {n}",
            app.cursor_song
        );
        assert!(app.view_albums.is_empty() || app.cursor_album < app.view_albums.len());
        assert!(app.view_artists.is_empty() || app.cursor_artist < app.view_artists.len());
        for (pos, &i) in app.view_songs.iter().enumerate() {
            assert!(i < app.tracks.len());
            assert_eq!(app.song_pos[i], pos);
        }
        if let Some(p) = app.playing {
            assert!(p < app.tracks.len());
        }
        assert!((-50..=100).contains(&app.speed_pct));
    }
    #[test]
    fn library_groups_into_albums_and_artists() {
        let app = app_with("groups", library());
        assert_eq!(app.tracks.len(), 12);
        assert_eq!(app.albums.len(), 4);
        assert_eq!(app.artists.len(), 3);
        assert_eq!(app.view_songs.len(), 12);
        invariants(&app);
    }
    #[test]
    fn search_starts_at_top_when_playing_track_is_filtered_out() {
        let mut app = app_with("search-top", library());
        app.play_index(0);
        app.cursor_song = 9;
        app.offsets = [9, 2, 1];
        app.search = "sasha".into();
        app.apply_search();
        assert_eq!(app.view_songs.len(), 3);
        assert_eq!(app.cursor_song, 0);
        assert_eq!(app.offsets, [0, 0, 0]);
        invariants(&app);
    }
    #[test]
    fn search_follows_playing_track_when_it_matches() {
        let mut app = app_with("search-follow", library());
        let sasha = app
            .tracks
            .iter()
            .position(|t| t.title == "Xpander")
            .unwrap();
        app.play_index(sasha);
        app.search = "sasha".into();
        app.apply_search();
        assert_eq!(app.view_songs[app.cursor_song], sasha);
        assert_eq!(app.offsets, [0, 0, 0]);
    }
    #[test]
    fn empty_search_shows_everything_and_matches_are_case_insensitive() {
        let mut app = app_with("search-empty", library());
        app.search = "ALLY".into();
        app.apply_search();
        assert_eq!(app.view_songs.len(), 1);
        app.search = "  ".into();
        app.apply_search();
        assert_eq!(app.view_songs.len(), 12);
        assert_eq!(app.view_albums.len(), 4);
        assert_eq!(app.view_artists.len(), 3);
    }
    #[test]
    fn artist_track_sort_orders_by_artist_then_title_then_album() {
        let mut app = app_with("sort-at", library());
        app.sort_row = SortKey::ALL
            .iter()
            .position(|&k| k == SortKey::ArtistTrack)
            .unwrap();
        app.apply_sort();
        assert_eq!(app.sort, SortKey::ArtistTrack);
        let keys: Vec<(String, String, String)> = app
            .view_songs
            .iter()
            .map(|&i| {
                let t = &app.tracks[i];
                (
                    t.artist.to_lowercase(),
                    t.title.to_lowercase(),
                    t.album.to_lowercase(),
                )
            })
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert_eq!(app.tracks[app.view_songs[0]].title, "Avril 14th");
        assert_eq!(app.tracks[app.view_songs[1]].title, "Minipops");
        assert_eq!(app.offsets[0], 0);
    }
    #[test]
    fn every_sort_key_is_a_permutation() {
        let mut app = app_with("sort-perm", library());
        for (row, _) in SortKey::ALL.iter().enumerate() {
            app.sort_row = row;
            app.apply_sort();
            let mut v = app.view_songs.clone();
            v.sort();
            assert_eq!(v, (0..12).collect::<Vec<_>>());
            invariants(&app);
        }
    }
    #[test]
    fn scroll_offset_keeps_cursor_visible_with_minimal_movement() {
        // cursor already visible: offset unchanged, recenter or not
        assert_eq!(App::scroll_offset(10, 14, 100, 10, false), 10);
        assert_eq!(App::scroll_offset(10, 14, 100, 10, true), 10);
        // user moved one row past the bottom: scroll by one
        assert_eq!(App::scroll_offset(10, 20, 100, 10, false), 11);
        // and one row above the top
        assert_eq!(App::scroll_offset(10, 9, 100, 10, false), 9);
        // offset beyond the list is pulled back
        assert_eq!(App::scroll_offset(500, 3, 20, 10, false), 3);
        // zero height is treated as one row
        assert_eq!(App::scroll_offset(0, 7, 20, 0, false), 7);
    }
    #[test]
    fn scroll_offset_centres_off_screen_track_changes() {
        // shuffle jumped far down: cursor lands mid screen
        assert_eq!(App::scroll_offset(0, 50, 100, 10, true), 45);
        // and far up
        assert_eq!(App::scroll_offset(80, 20, 100, 10, true), 15);
        // just one past the bottom edge is still centred (it was off screen)
        assert_eq!(App::scroll_offset(10, 20, 100, 10, true), 15);
        // clamped at the ends of the list
        assert_eq!(App::scroll_offset(50, 2, 100, 10, true), 0);
        assert_eq!(App::scroll_offset(0, 98, 100, 10, true), 90);
        // list shorter than the screen never scrolls
        assert_eq!(App::scroll_offset(0, 4, 5, 10, true), 0);
    }
    #[test]
    fn track_changes_request_recentre_but_cursor_moves_do_not() {
        let mut app = app_with("recentre", library());
        assert_eq!(app.recenter, [false; 3]);
        app.play_index(5);
        assert_eq!(app.recenter, [true; 3]);
        app.recenter = [false; 3];
        app.toggle_shuffle();
        app.step_track(1);
        assert_eq!(app.recenter, [true; 3]);
        app.section = Section::Songs;
        app.move_cursor(1);
        assert_eq!(app.recenter, [false, true, true]);
        app.section = Section::Albums;
        app.move_cursor(-1);
        assert_eq!(app.recenter, [false, false, true]);
    }
    #[test]
    fn step_track_wraps_both_ways() {
        let mut app = app_with("step", library());
        app.play_index(11);
        app.step_track(1);
        assert_eq!(app.playing, Some(app.view_songs[0]));
        app.step_track(-1);
        assert_eq!(app.playing, Some(app.view_songs[11]));
    }
    #[test]
    fn step_track_without_tracks_is_a_noop() {
        let mut app = app_with("step-empty", Vec::new());
        app.step_track(1);
        app.toggle_shuffle();
        app.step_track(1);
        app.step_track(-1);
        assert_eq!(app.playing, None);
    }
    #[test]
    fn shuffle_never_repeats_current_and_previous_walks_history() {
        let mut app = app_with("shuffle", library());
        app.toggle_shuffle();
        assert!(app.shuffle);
        assert_eq!(app.status, "Shuffle on");
        app.play_index(0);
        let mut seen = vec![0usize];
        for _ in 0..200 {
            let before = app.playing.unwrap();
            app.step_track(1);
            let after = app.playing.unwrap();
            assert_ne!(before, after, "shuffle repeated the current track");
            assert!(app.view_songs.contains(&after));
            seen.push(after);
        }
        let mut distinct = seen.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 12);
        for expected in seen.iter().rev().skip(1).take(50) {
            app.step_track(-1);
            assert_eq!(app.playing, Some(*expected));
        }
        app.toggle_shuffle();
        assert!(!app.shuffle);
        assert_eq!(app.status, "Shuffle off");
    }
    #[test]
    fn shuffle_with_one_visible_track_falls_back_to_linear() {
        let mut app = app_with("shuffle-one", library());
        app.search = "ally pally".into();
        app.apply_search();
        app.toggle_shuffle();
        app.play_index(app.view_songs[0]);
        app.step_track(1);
        assert_eq!(app.playing, Some(app.view_songs[0]));
    }
    #[test]
    fn speed_adjusts_clamps_and_resets() {
        let mut app = app_with("speed", library());
        app.adjust_speed(30);
        assert_eq!(app.speed_pct, 30);
        assert!((app.speed() - 1.3).abs() < 1e-6);
        app.adjust_speed(100);
        assert_eq!(app.speed_pct, 100);
        app.adjust_speed(-200);
        assert_eq!(app.speed_pct, -50);
        app.reset_speed();
        assert_eq!(app.speed_pct, 0);
        assert_eq!(app.speed(), 1.0);
    }
    #[test]
    fn tape_modes_expose_hiss_row_and_clamp_it() {
        let mut app = app_with("hiss", library());
        app.fx.mode = Mode::Cassette;
        let rows = app.fx_rows();
        assert!(rows.contains(&FxRow::Hiss));
        assert!(!rows.contains(&FxRow::Scratch));
        app.fx_row = rows.iter().position(|&r| r == FxRow::Hiss).unwrap();
        for _ in 0..30 {
            app.fx_adjust(5);
        }
        assert_eq!(app.fx.hiss, 100);
        for _ in 0..60 {
            app.fx_adjust(-5);
        }
        assert_eq!(app.fx.hiss, 0);
        app.fx_reset_row();
        assert_eq!(app.fx.hiss, FxParams::default().hiss);
        app.fx.mode = Mode::Vinyl;
        assert!(!app.fx_rows().contains(&FxRow::Hiss));
    }
    #[test]
    fn fx_mode_row_cycles_through_all_modes() {
        let mut app = app_with("fx-mode", library());
        app.fx_row = 0;
        let start = app.fx.mode;
        for _ in 0..Mode::ALL.len() {
            app.fx_adjust(5);
        }
        assert_eq!(app.fx.mode, start);
        app.fx_adjust(-5);
        assert_eq!(app.fx.mode, *Mode::ALL.last().unwrap());
    }
    #[test]
    fn move_cursor_clamps_at_ends() {
        let mut app = app_with("cursor", library());
        app.move_cursor(-5);
        assert_eq!(app.cursor_song, 0);
        app.move_cursor(i64::MAX / 2);
        assert_eq!(app.cursor_song, 11);
        app.section = Section::Albums;
        app.move_cursor(i64::MIN / 2);
        assert_eq!(app.cursor_album, 0);
        invariants(&app);
    }
    #[test]
    #[ignore]
    fn soak_random_operations_keep_invariants() {
        let mut lib = Vec::new();
        for ar in 0..120 {
            for al in 0..6 {
                for n in 1..=12 {
                    lib.push(track(
                        &format!("Artist {ar:03}"),
                        &format!("Album {al}"),
                        &format!("Song {:03}", (ar * 7 + al * 3 + n) % 97),
                        n,
                    ));
                }
            }
        }
        let n_tracks = lib.len();
        let mut app = app_with("soak", lib);
        let mut rng = 0x1234_5678_9abc_def1u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let words = [
            "artist 00",
            "album 3",
            "song 01",
            "zzz",
            "",
            "ARTIST 119",
            "7",
        ];
        let started = TInstant::now();
        let iters = 200_000;
        for _ in 0..iters {
            match next() % 14 {
                0 => app.move_cursor((next() % 41) as i64 - 20),
                1 => app.step_track(1),
                2 => app.step_track(-1),
                3 => app.toggle_shuffle(),
                4 => {
                    app.search = words[(next() % words.len() as u64) as usize].into();
                    app.apply_search();
                }
                5 => {
                    app.sort_row = (next() % SortKey::ALL.len() as u64) as usize;
                    app.apply_sort();
                }
                6 => app.play_index((next() % n_tracks as u64) as usize),
                7 => app.adjust_speed((next() % 21) as i32 - 10),
                8 => app.reset_speed(),
                9 => {
                    app.section = match next() % 3 {
                        0 => Section::Songs,
                        1 => Section::Albums,
                        _ => Section::Artists,
                    }
                }
                10 => app.activate_cursor(),
                11 => app.toggle_play(),
                12 => {
                    app.fx_row = (next() % app.fx_rows().len() as u64) as usize;
                    app.fx_adjust((next() % 11) as i32 - 5);
                }
                _ => app.flush_play(),
            }
            invariants(&app);
        }
        let el = started.elapsed();
        eprintln!(
            "soak: {iters} ops over {n_tracks} tracks in {:.2}s ({:.0} ops/s)",
            el.as_secs_f64(),
            iters as f64 / el.as_secs_f64()
        );
    }
    #[test]
    #[ignore]
    fn perf_search_and_sort_on_large_library() {
        let mut lib = Vec::new();
        for i in 0..20_000u32 {
            lib.push(track(
                &format!("Artist {:04}", i % 1500),
                &format!("Album {}", i % 4000),
                &format!("Title {}", (i * 2654435761u32.wrapping_mul(1)) % 100_000),
                i % 15 + 1,
            ));
        }
        let mut app = app_with("perf", lib);
        let t = TInstant::now();
        for q in ["artist 07", "title 1", "album 39", ""] {
            app.search = q.into();
            app.apply_search();
        }
        let search_ms = t.elapsed().as_secs_f64() * 1000.0 / 4.0;
        let t = TInstant::now();
        for row in 0..SortKey::ALL.len() {
            app.sort_row = row;
            app.apply_sort();
        }
        let sort_ms = t.elapsed().as_secs_f64() * 1000.0 / SortKey::ALL.len() as f64;
        eprintln!("perf: 20k tracks, search {search_ms:.1} ms, sort+filter {sort_ms:.1} ms");
        assert!(search_ms < 200.0, "search {search_ms} ms");
        assert!(sort_ms < 500.0, "sort {sort_ms} ms");
    }
}
