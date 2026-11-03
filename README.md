# vinyl

A terminal music player. Point it at a folder of audio files and it indexes them (tags via lofty, playback via rodio) and lets you browse by song, album, or artist.

## Setup

    cargo install --path .
    vinyl

Runs on macOS, Linux and Windows. Audio goes through the platform's native API (CoreAudio, ALSA, WASAPI). On Linux the build needs the ALSA headers and pkg-config, e.g. `sudo apt install libasound2-dev pkg-config`. On macOS, if your library lives in `~/Music/Music` or `~/Music/iTunes`, give your terminal app access under System Settings → Privacy & Security → Media & Apple Music.

On first run `~/.vinyl` is created with:

    music.home = ~/Music

Set `music.home` to the folder holding your music; every audio file at or under it is indexed. The index is cached in `~/.vinyl-index.json` (override with `index.file = ...`) and refreshed on each start, re-reading tags only for files whose size or modification time changed. `VINYL_CONFIG=/path/to/file` points at a different config file.

Supported formats: mp3, flac, m4a/aac, ogg/vorbis, opus, wav, aiff.

## Keys

| Key | Action |
| --- | --- |
| `s` / `a` / `r` | Show the song / album / artist list |
| `/` | Search: type text matched against artist, album and title, then `Enter`. An empty search shows everything again; `Esc` cancels |
| `o` | Sort the song list by artist, track title, or album (`a`/`t`/`b` are shortcuts inside the popup). Next/previous follow the chosen order |
| `↑` / `↓` (or `k` / `j`) | Move the selection; `PgUp` / `PgDn` move a page, `Home` / `End` jump to the top / bottom |
| `Enter` | Play the selected song, or the first track of the selected album / artist |
| `space` | Play / pause (plays the selection if nothing is loaded) |
| `n` / `p` | Next / previous track |
| `z` | Toggle shuffle: next and auto-advance pick a random song from the current list, previous walks back through what was played |
| `←` / `→` (or `,` / `.`) | Rewind / fast-forward 5 seconds |
| `=` (or `+`) / `-` | Playback speed +1% / −1% (range 50% to 150%) |
| `0` | Reset speed to 100% |
| `x` | FX: vinyl & tape effects dialog (see below) |
| `e` | Equaliser & stereo dialog: off, 3-band or 7-band EQ, loudness, mono / stereo width |
| `l` | Toggle loudness (bass and treble lift for quiet listening) |
| `f` | Reveal the current file in Finder / Explorer / your file manager |
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
| Reel-to-reel | Tape age, Saturation, Wow, Flutter | wide bandwidth, light hiss, gentle tanh saturation, slow wow |
| Cassette | Tape age, Saturation, Wow, Flutter | 12 kHz bandwidth, more hiss, stronger wow and flutter |

Tape age lowers the bandwidth, raises hiss, adds extra wow, and introduces occasional dropouts.

## Equaliser and loudness

Press `e` for the equaliser. The 3-band mode has a low shelf at 200 Hz, a mid peak at 1 kHz and a high shelf at 4 kHz; the 7-band mode is a graphic EQ at 60, 150, 400, 1k, 2.5k, 6k and 15k Hz. Each band ranges ±12 dB (`←`/`→` for 1 dB, `PgUp`/`PgDn` for 3 dB, `0` to flatten). The last row toggles loudness, which `l` also toggles from the main screen. A soft limiter after the EQ keeps boosts from clipping.

The Stereo row offers Normal, Mono, and Width. Width uses mid/side processing: the side signal (L−R) is scaled from 0% to 200%, where 100% leaves the image untouched and higher values push it wider (`←`/`→` for 5%, `PgUp`/`PgDn` for 15%). EQ, loudness and stereo settings are saved alongside the tape settings.

## Versions

### v1

- initial version

### v1.1

- added shuffle
- changed fx/speed keys
- added --probe
- fix for old files encoded with RIFF and MP3

