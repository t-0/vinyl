use super::settings::settings_file;
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
fn shuffle_is_remembered_across_restarts() {
    let config = temp_config("shuffle-persist");
    let mut app = App::build(
        temp_config("shuffle-persist"),
        FxParams::default(),
        None,
        None,
    );
    assert!(!load_settings(&config).shuffle);
    app.toggle_shuffle();
    assert!(load_settings(&config).shuffle);
    assert_eq!(load_settings(&config).fx, FxParams::default());
    app.toggle_shuffle();
    assert!(!load_settings(&config).shuffle);
}
#[test]
fn settings_file_without_shuffle_still_loads_fx() {
    let config = temp_config("settings-compat");
    let fx = FxParams {
        preamp: -6,
        loudness: true,
        ..FxParams::default()
    };
    // The layout older versions wrote: FxParams alone at the top level.
    std::fs::write(settings_file(&config), serde_json::to_string(&fx).unwrap()).unwrap();
    let s = load_settings(&config);
    assert_eq!(s.fx, fx);
    assert!(!s.shuffle);
}
#[test]
fn last_track_is_remembered_and_restored_paused() {
    let config = temp_config("last-track");
    let mut app = app_with("last-track", library());
    assert_eq!(load_settings(&config).last_track, None);
    app.play_index(5);
    app.flush_play();
    let path = app.tracks[5].path.clone();
    assert_eq!(load_settings(&config).last_track, Some(path.clone()));
    // Closing a dialog with nothing playing yet must not forget it.
    let mut fresh = App::build(temp_config("last-track"), FxParams::default(), None, None);
    fresh.resume_track = load_settings(&config).last_track;
    fresh.close_fx();
    assert_eq!(load_settings(&config).last_track, Some(path.clone()));
    // The index arrives: the track is cued, paused, with the cursor on it.
    let mut tracks = library();
    tracks.sort_by_cached_key(|t| t.sort_key());
    fresh.set_tracks(tracks);
    let i = fresh.tracks.iter().position(|t| t.path == path).unwrap();
    assert_eq!(fresh.playing, Some(i));
    assert!(fresh.start_paused);
    assert_eq!(fresh.resume_track, None);
    assert_eq!(fresh.cursor_track(), Some(i));
    fresh.flush_play();
    assert!(!fresh.start_paused);
    // A re-index later does not restore anything again.
    fresh.playing = None;
    let mut tracks = library();
    tracks.sort_by_cached_key(|t| t.sort_key());
    fresh.set_tracks(tracks);
    assert_eq!(fresh.playing, None);
}
#[test]
fn speed_is_remembered_and_clamped_on_restore() {
    let config = temp_config("speed-persist");
    let mut app = app_with("speed-persist", library());
    assert_eq!(load_settings(&config).speed_pct, 0);
    app.adjust_speed(30);
    assert_eq!(load_settings(&config).speed_pct, 30);
    app.reset_speed();
    assert_eq!(load_settings(&config).speed_pct, 0);
    app.adjust_speed(-20);
    assert_eq!(load_settings(&config).speed_pct, -20);
    // Restoring goes through the same clamp as the keys.
    let mut fresh = App::build(
        temp_config("speed-persist"),
        FxParams::default(),
        None,
        None,
    );
    fresh.set_speed_pct(load_settings(&config).speed_pct);
    assert_eq!(fresh.speed_pct, -20);
    fresh.set_speed_pct(500);
    assert_eq!(fresh.speed_pct, 100);
}
#[test]
fn missing_last_track_is_ignored() {
    let mut app = App::build(
        temp_config("last-track-gone"),
        FxParams::default(),
        None,
        None,
    );
    app.resume_track = Some(PathBuf::from("/m/gone/gone/01 gone.mp3"));
    let mut tracks = library();
    tracks.sort_by_cached_key(|t| t.sort_key());
    app.set_tracks(tracks);
    assert_eq!(app.playing, None);
    assert!(!app.start_paused);
    assert_eq!(app.resume_track, None);
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
    app.fx.mode = FxMode::Cassette;
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
    app.fx.mode = FxMode::Vinyl;
    assert!(!app.fx_rows().contains(&FxRow::Hiss));
}
#[test]
fn fx_mode_row_cycles_through_all_modes() {
    let mut app = app_with("fx-mode", library());
    app.fx_row = 0;
    let start = app.fx.mode;
    for _ in 0..FxMode::ALL.len() {
        app.fx_adjust(5);
    }
    assert_eq!(app.fx.mode, start);
    app.fx_adjust(-5);
    assert_eq!(app.fx.mode, *FxMode::ALL.last().unwrap());
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
