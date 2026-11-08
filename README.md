# vinyl

A music player for the terminal, or for a desktop window that looks just like it. Point it at a folder of audio files and it indexes them (tags via lofty, playback via rodio) and lets you browse by song, album, or artist.

## Setup

    cargo install --path .
    vinyl           # in the terminal
    vinyl --gui     # in a window

Runs on macOS, Linux and Windows. Audio goes through the platform's native API (CoreAudio, ALSA, WASAPI). On Linux the build needs the ALSA headers and pkg-config, e.g. `sudo apt install libasound2-dev pkg-config`. On macOS, if your library lives in `~/Music/Music` or `~/Music/iTunes`, give your terminal app access under System Settings → Privacy & Security → Media & Apple Music.

### Terminal or window

The same player runs either way; `--gui` opens a desktop window (built on [GPUI](https://gpui.rs), Zed's UI framework) with the same layout, colours, dialogs and keys as the terminal version, plus mouse clicks on the tabs and list rows (double-click plays) and scroll-wheel scrolling. `--no-gui` forces the terminal. Without a flag the `ui` setting in `~/.vinyl` decides, and it defaults to the terminal:

    ui = gui

The window is a Cargo feature, `gui`, that is on by default. It pulls in a large GPU and windowing stack, so a terminal-only build is much quicker:

    cargo install --path . --no-default-features

On Linux the window needs a Wayland or X11 session and these development packages at build time (Debian/Ubuntu names): `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev libx11-dev libxcb1-dev`. A `cli` audio editor (see below) cannot take over the window, so when started from the window it runs in whatever terminal vinyl was launched from while playback carries on.

On first run `~/.vinyl` is created with:

    music.home = ~/Music

Set `music.home` to the folder holding your music; every audio file at or under it is indexed. The index is cached in `~/.vinyl-index.json` (override with `index.file = ...`) and refreshed on each start, re-reading tags only for files whose size or modification time changed. `VINYL_CONFIG=/path/to/file` points at a different config file. `VINYL_AUDIO_BUFFER=4096` asks the output device for a fixed period of that many frames instead of leaving the choice to the audio backend; useful on small devices where the audio thread gets no real-time priority.

### Audio editor

    audio.editor = /Applications/Audacity.app
    audio.editor.mode = gui

`audio.editor` is the command that `E` runs on the selected track. Write `{file}` where the track path should go (it may appear more than once, and inside a longer argument such as `--in={file}`); if the command has no `{file}`, the path is appended as the last argument. Quote arguments that contain spaces, e.g. `"Logic Pro.app"`. On macOS a `.app` bundle is launched through `open -a`. `audio.editor.mode` says how the editor behaves:

- `gui` (default): the editor is a desktop app. vinyl starts it and carries on playing.
- `cli`: the editor runs in this terminal. vinyl pauses playback, hands the screen over, waits for the editor to exit, then redraws, resumes playback and refreshes the index so edited tags or lengths show up. Your selection and scroll position are kept.

Supported formats: mp3, flac, m4a/aac, ogg/vorbis, opus, wav, aiff.

## Keys

| Key | Action |
| --- | --- |
| `s` / `a` / `r` | Show the song / album / artist list |
| `/` | Search: type text matched against artist, album and title, then `Enter`. An empty search shows everything again; `Esc` cancels |
| `o` | Sort the song list by artist, track title, album, or artist then track title (`a`/`t`/`b`/`r` are shortcuts inside the popup). Next/previous follow the chosen order |
| `↑` / `↓` (or `k` / `j`) | Move the selection; `PgUp` / `PgDn` move a page, `Home` / `End` jump to the top / bottom |
| `Enter` | Play the selected song, or the first track of the selected album / artist |
| `space` | Play / pause (plays the selection if nothing is loaded) |
| `n` / `p` | Next / previous track |
| `z` | Toggle shuffle: next and auto-advance pick a random song from the current list, previous walks back through what was played |
| `←` / `→` (or `,` / `.`) | Rewind / fast-forward 5 seconds |
| `=` (or `+`) / `-` | Playback speed +1% / −1% (range 50% to 150%) |
| `0` | Reset speed to 100% |
| `x` | FX: vinyl & tape effects dialog (see below) |
| `e` | Equaliser & stereo dialog: preamp, auto-normalise, off, 3-band or 7-band EQ, loudness, mono / stereo width |
| `l` | Toggle loudness (bass and treble lift for quiet listening) |
| `f` | Reveal the current file in Finder / Explorer / your file manager |
| `E` | Open the selected track in your audio editor (see `audio.editor` above; the key is only shown once one is configured) |
| `i` | Rebuild the index from scratch |
| `q` | Quit |

Speed changes are done by resampling, so pitch shifts with speed.

MP3 files that iTunes or QuickTime wrapped in a RIFF/WAVE header (a WAV file whose payload is MP3 frames) are detected and the MP3 payload is decoded directly.

If a file will not play, `vinyl --probe <file>` opens it the same way playback does and prints the decoder's view of it (format, duration, how much audio the first five seconds yield) or the error.

## Vinyl & tape effects

Press `x` to open the effects dialog. Pick a mode with `←`/`→` on the Mode row, move between rows with `↑`/`↓`, and adjust a value with `←`/`→` (±5) or `+`/`-` (±1). `0` resets the row, `Esc` closes. Changes apply immediately to whatever is playing and are remembered in `~/.vinyl-fx.json`.

| Mode | Controls | Character |
| --- | --- | --- |
| Vinyl | Scratch, Distortion | random crackle and pops, surface noise, warm asymmetric distortion, 16 kHz roll-off |
| Reel-to-reel | Tape age, Hiss, Saturation, Wow, Flutter | wide bandwidth, light hiss, gentle tanh saturation, slow wow |
| Cassette | Tape age, Hiss, Saturation, Wow, Flutter | 12 kHz bandwidth, more hiss, stronger wow and flutter |

Tape age lowers the bandwidth, raises hiss, adds extra wow, and introduces occasional dropouts. Hiss sets the base noise level independently: 0 is silent, 50 is the natural level for the mode, and age still scales it.

## Equaliser and loudness

Press `e` for the equaliser. The first row is the preamp: a gain from −24 dB to +12 dB applied before everything else (tape and vinyl effects, EQ, loudness, stereo), so it sets how hard the signal drives the tape saturation and vinyl distortion as well as the overall level. `←`/`→` move it 1 dB, `PgUp`/`PgDn` 3 dB, `0` returns it to 0 dB. A boost is soft-limited so it cannot clip.

The Normalise row below it levels tracks automatically. When it is on, the first time a track plays vinyl decodes it in the background and measures its loudness (RMS over 400 ms blocks, ignoring near-silent blocks), then adds whatever gain brings it to −18 dBFS, between −18 dB and +12 dB, to the preamp. The measurement takes a moment on first play, fades in when it arrives, and is stored in the index so later plays start at the right level; a file that changes is measured again. The Preamp row shows the track's gain next to your own setting, and `vinyl --probe <file>` prints it too.

The 3-band mode has a low shelf at 200 Hz, a mid peak at 1 kHz and a high shelf at 4 kHz; the 7-band mode is a graphic EQ at 60, 150, 400, 1k, 2.5k, 6k and 15k Hz. Each band ranges ±12 dB (`←`/`→` for 1 dB, `PgUp`/`PgDn` for 3 dB, `0` to flatten). The last row toggles loudness, which `l` also toggles from the main screen. A soft limiter after the EQ keeps boosts from clipping.

The Stereo row offers Normal, Mono, and Width. Width uses mid/side processing: the side signal (L−R) is scaled from 0% to 200%, where 100% leaves the image untouched and higher values push it wider (`←`/`→` for 5%, `PgUp`/`PgDn` for 15%). EQ, loudness and stereo settings are saved alongside the tape settings.

## Testing

`cargo test` runs the unit tests: library grouping, search and sort behaviour, shuffle, speed, FX parameter handling, the key bindings shared by both front ends, the MP3-in-WAV unwrapping and slice reader, and a guard that the FX chain runs faster than real time in a debug build. None of them touch an audio device or open a window; `cargo test --no-default-features` runs the same tests without building the GUI stack.

Longer soak and performance tests are marked `#[ignore]` and are meant for a release build:

    cargo test --release -- --ignored --nocapture

They push 200k random operations through the app state against an 8k-track library, stream an hour of audio through the FX chain with parameters randomised twice a second, decode a ten-minute file with a thousand random seeks, and time search and sort over 20k tracks. Each prints its throughput.

## Versions

### v1

- initial version

### v1.1

- added shuffle
- changed fx/speed keys
- added --probe
- fix for old files encoded with RIFF and MP3

### v1.2

- added artist/track sort order
- correct list view when applying search
- added tape hiss parameter
- simplify hints on now playing

### v1.4

- added tests including soak tests

### v1.5

- error message leakage - added screen refresh and removing screen pollution (anything the audio stack writes to stderr now lands in `~/.vinyl.log` instead of on the screen)
- center jumped-to tracks

### v1.6

- added `audio.editor` and `audio.editor.mode` settings and the `E` key to open the selected track in an audio editor
- the file path under the analyser is shown relative to `music.home`
- added a preamp (−24 dB to +12 dB) ahead of the FX chain, in the equaliser dialog
- added auto-normalisation: per-track level measured on first play, cached in the index, added to the preamp
- flag unknown configuration keys

### v1.7

- audio buffer handling for desktop and device

### v1.8

- added a desktop window with the same look, dialogs and keys as the terminal UI, built on GPUI: `vinyl --gui`, `--no-gui`, and a `ui = tui|gui` setting
- key handling is shared by both front ends (`src/keys.rs`); the terminal loop moved to `src/tui.rs`
- the window is the `gui` Cargo feature (on by default); `--no-default-features` builds the terminal-only player
