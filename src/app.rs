use crate::audio::Audio;
use crate::config::{Config, display_path};
use crate::fx::{EQ_RANGE_DB, EqMode, FxParams, Mode, StereoMode, WIDTH_MAX};
use crate::index::{self, IndexMsg, Track};
use crate::spectrum::Analyser;
use anyhow::Result;
use std::process::Command;
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
    pub speed_pct: i32,
    pub shuffle: bool,
    shuffle_rng: u64,
    shuffle_history: Vec<usize>,
    pub audio: Option<Audio>,
    pub audio_error: Option<String>,
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
    pub should_quit: bool,
}
impl App {
    pub fn new(config: Config) -> Self {
        let fx = load_fx(&config);
        let (audio, audio_error) = match Audio::new(fx) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let mut app = Self {
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
            should_quit: false,
        };
        app.start_index(false);
        app
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
        // Start every filtered view at the top, unless the playing track is in it.
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
        // xorshift64*
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
        self.speed_pct = (self.speed_pct + delta).clamp(-50, 50);
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
    pub fn fx_rows(&self) -> Vec<FxRow> {
        let mut rows = vec![FxRow::Mode];
        match self.fx.mode {
            Mode::Off => {}
            Mode::Vinyl => rows.extend([FxRow::Scratch, FxRow::Distortion]),
            Mode::ReelToReel | Mode::Cassette => {
                rows.extend([
                    FxRow::Age,
                    FxRow::Hiss,
                    FxRow::Saturation,
                    FxRow::Wow,
                    FxRow::Flutter,
                ])
            }
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
        let mut rows = vec![EqRow::Mode];
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
    pub fn close_eq(&mut self) {
        self.eq_open = false;
        self.save_settings();
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
    pub fn tick(&mut self) -> Result<()> {
        self.poll_index();
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EqRow {
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
    if cfg!(target_os = "macos") {
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
    }
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
