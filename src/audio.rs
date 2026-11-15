use crate::fx::{Fx, FxParams, NORMALIZE_RANGE_DB, NORMALIZE_TARGET_DB, SharedParams};
use crate::spectrum::{SharedTap, Tap};
use anyhow::{Context, Result};
use rodio::cpal::BufferSize;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
/// Decoder read-ahead. Decoding runs on the audio callback thread, so every
/// refill is a read from disk on the thread that must not stall; a bigger
/// buffer means fewer of them.
const DECODER_BUFFER: usize = 64 * 1024;
/// Frames per output period, when `VINYL_AUDIO_BUFFER` asks for a fixed one.
/// Otherwise the audio backend's default applies. A small device without
/// real-time scheduling needs more room than a desktop; the player sets this.
fn buffer_size() -> Option<u32> {
    std::env::var("VINYL_AUDIO_BUFFER")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
}
fn open_output() -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
    // `from_default_device` picks a fixed 50 ms buffer, which CoreAudio
    // refuses on devices with a smaller maximum (a USB DAC on a Mac allows
    // 1024 frames); `BufferSize::Default` leaves the period to the backend,
    // which is what a desktop wants. A player that cannot live with the
    // backend's default (on ALSA the driver's smallest period) sets
    // `VINYL_AUDIO_BUFFER` and gets a fixed one.
    let first = DeviceSinkBuilder::from_default_device().and_then(|b| {
        let size = match buffer_size() {
            Some(frames) => BufferSize::Fixed(frames),
            None => BufferSize::Default,
        };
        b.with_buffer_size(size).open_stream()
    });
    match first {
        Ok(s) => Ok(s),
        Err(_) => DeviceSinkBuilder::open_default_sink(),
    }
}
pub struct Audio {
    _stream: MixerDeviceSink,
    player: Player,
    fx: SharedParams,
    tap: SharedTap,
}
pub struct FileSlice {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
}
impl FileSlice {
    fn new(mut file: File, start: u64, len: u64) -> std::io::Result<Self> {
        file.seek(SeekFrom::Start(start))?;
        Ok(Self {
            file,
            start,
            len,
            pos: 0,
        })
    }
}
impl Read for FileSlice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let max = (self.len - self.pos).min(buf.len() as u64) as usize;
        let n = self.file.read(&mut buf[..max])?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for FileSlice {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start of slice",
            ));
        }
        let target = target as u64;
        self.file.seek(SeekFrom::Start(self.start + target))?;
        self.pos = target;
        Ok(target)
    }
}
pub fn locate_audio(path: &Path) -> Result<(File, u64, u64, Option<String>)> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let file_len = file.metadata()?.len();
    let mut head = [0u8; 10];
    let mut off = 0u64;
    if file.read(&mut head)? == 10 && head.starts_with(b"ID3") {
        let sz = ((head[6] as u64 & 0x7f) << 21)
            | ((head[7] as u64 & 0x7f) << 14)
            | ((head[8] as u64 & 0x7f) << 7)
            | (head[9] as u64 & 0x7f);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        off = 10 + sz + footer;
    }
    let mut riff = [0u8; 12];
    file.seek(SeekFrom::Start(off))?;
    if off < file_len
        && file.read(&mut riff)? == 12
        && &riff[0..4] == b"RIFF"
        && &riff[8..12] == b"WAVE"
    {
        let mut pos = off + 12;
        let mut fmt_tag = None;
        let mut data = None;
        let mut hdr = [0u8; 8];
        while pos + 8 <= file_len && pos < off + 1024 * 1024 {
            file.seek(SeekFrom::Start(pos))?;
            if file.read(&mut hdr)? < 8 {
                break;
            }
            let size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
            match &hdr[0..4] {
                b"fmt " => {
                    let mut tag = [0u8; 2];
                    file.read_exact(&mut tag)?;
                    fmt_tag = Some(u16::from_le_bytes(tag));
                }
                b"data" => {
                    data = Some((pos + 8, size));
                    break;
                }
                _ => {}
            }
            pos += 8 + size + (size & 1);
        }
        if let (Some(tag @ (0x50 | 0x55)), Some((start, size))) = (fmt_tag, data) {
            let avail = file_len.saturating_sub(start);
            let len = if size == 0 || size > avail {
                avail
            } else {
                size
            };
            let note = format!(
                "RIFF/WAVE wrapper with MPEG payload (format tag 0x{tag:02x}); decoding data chunk at offset {start}, {len} bytes"
            );
            return Ok((file, start, len, Some(note)));
        }
    }
    Ok((file, 0, file_len, None))
}
pub fn open_decoder(path: &Path) -> Result<Decoder<BufReader<FileSlice>>> {
    let (file, start, len, note) = locate_audio(path)?;
    let slice = FileSlice::new(file, start, len)?;
    let mut builder = Decoder::builder()
        .with_data(BufReader::with_capacity(DECODER_BUFFER, slice))
        .with_seekable(true)
        .with_byte_len(len);
    let ext = path.extension().and_then(|e| e.to_str());
    if let Some(ext) = ext {
        builder = builder.with_hint(if note.is_some() { "mp3" } else { ext });
    }
    builder
        .build()
        .with_context(|| format!("decoding {}", path.display()))
}
/// Decodes the whole file and returns the gain in dB that would bring its
/// level to `NORMALIZE_TARGET_DB`. Level is the RMS over 400 ms blocks,
/// ignoring blocks quieter than -70 dBFS so silence at the ends and in
/// pauses does not make a track look quieter than it is.
pub fn measure_gain(path: &Path) -> Result<f32> {
    use rodio::Source;
    let dec = open_decoder(path)?;
    let block = (dec.sample_rate().get() as usize * dec.channels().get() as usize * 2) / 5;
    Ok(gain_for(block_levels(dec, block.max(1))))
}
/// Mean square of each `block`-sample block.
fn block_levels(samples: impl Iterator<Item = f32>, block: usize) -> Vec<f32> {
    let mut levels = Vec::new();
    let (mut sum, mut n) = (0.0f64, 0usize);
    for v in samples {
        sum += (v as f64) * (v as f64);
        n += 1;
        if n == block {
            levels.push((sum / n as f64) as f32);
            sum = 0.0;
            n = 0;
        }
    }
    if n > block / 2 {
        levels.push((sum / n as f64) as f32);
    }
    levels
}
fn gain_for(levels: Vec<f32>) -> f32 {
    const GATE: f32 = 1e-7; // -70 dBFS as mean square
    let loud: Vec<f32> = levels.into_iter().filter(|&m| m > GATE).collect();
    if loud.is_empty() {
        return 0.0;
    }
    let mean = loud.iter().sum::<f32>() / loud.len() as f32;
    let level_db = 10.0 * mean.log10();
    (NORMALIZE_TARGET_DB - level_db).clamp(NORMALIZE_RANGE_DB.0, NORMALIZE_RANGE_DB.1)
}
pub fn probe(path: &Path, fx: FxParams) -> Result<String> {
    use rodio::Source;
    let mut out = String::new();
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    out.push_str(&format!(
        "file: {} ({} bytes)\n",
        path.display(),
        meta.len()
    ));
    out.push_str(&describe_header(path)?);
    if let Ok((_, _, _, Some(note))) = locate_audio(path) {
        out.push_str(&format!("unwrap: {note}\n"));
    }
    let dec = match open_decoder(path) {
        Ok(d) => d,
        Err(e) => {
            out.push_str(&format!("decoder: FAILED: {e:#}\n"));
            return Ok(out);
        }
    };
    let sr = dec.sample_rate().get();
    let ch = dec.channels().get();
    out.push_str(&format!(
        "decoder: {} Hz, {} ch, reported duration {:?}\n",
        sr,
        ch,
        dec.total_duration()
    ));
    match measure_gain(path) {
        Ok(g) => out.push_str(&format!(
            "normalise: {g:+.1} dB would bring it to {NORMALIZE_TARGET_DB} dBFS\n"
        )),
        Err(e) => out.push_str(&format!("normalise: FAILED: {e:#}\n")),
    }
    let mut raw = dec;
    let want = sr as usize * ch as usize * 5;
    let mut n = 0usize;
    let mut peak = 0f32;
    for s in raw.by_ref().take(want) {
        n += 1;
        peak = peak.max(s.abs());
    }
    out.push_str(&format!(
        "raw decode: {} samples in first 5 s (wanted {}), peak {:.3}\n",
        n, want, peak
    ));
    let dec = open_decoder(path)?;
    let mut fxs = Fx::new(dec, Arc::new(Mutex::new(fx)), None);
    let mut n = 0usize;
    let mut peak = 0f32;
    let mut bad = 0usize;
    for s in fxs.by_ref().take(want) {
        n += 1;
        if s.is_finite() {
            peak = peak.max(s.abs());
        } else {
            bad += 1;
        }
    }
    out.push_str(&format!(
        "through fx: {} samples, peak {:.3}, non-finite {}\n",
        n, peak, bad
    ));
    Ok(out)
}
fn describe_header(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut head = vec![0u8; 4 * 1024 * 1024];
    let n = f.read(&mut head)?;
    head.truncate(n);
    let mut out = String::new();
    let hex: Vec<String> = head.iter().take(16).map(|b| format!("{b:02x}")).collect();
    out.push_str(&format!("first bytes: {}\n", hex.join(" ")));
    let mut audio_start = 0usize;
    let mut other = false;
    if head.starts_with(b"ID3") && head.len() >= 10 {
        let sz = ((head[6] as usize & 0x7f) << 21)
            | ((head[7] as usize & 0x7f) << 14)
            | ((head[8] as usize & 0x7f) << 7)
            | (head[9] as usize & 0x7f);
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        audio_start = 10 + sz + footer;
        out.push_str(&format!(
            "ID3v2.{} tag: {} bytes (audio should start at offset {})\n",
            head[3], sz, audio_start
        ));
        if audio_start < head.len() {
            let hex: Vec<String> = head[audio_start..]
                .iter()
                .take(16)
                .map(|b| format!("{b:02x}"))
                .collect();
            out.push_str(&format!("after tag: {}\n", hex.join(" ")));
        }
    } else if head.starts_with(b"RIFF") {
        other = true;
        out.push_str("container: RIFF/WAVE wrapper (not a bare MP3)\n");
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        other = true;
        out.push_str(&format!(
            "container: MP4/M4A (brand {:?}) despite the .mp3 extension\n",
            String::from_utf8_lossy(&head[8..12.min(head.len())])
        ));
    } else if head.starts_with(b"fLaC") {
        other = true;
        out.push_str("container: FLAC\n");
    } else if head.starts_with(b"OggS") {
        other = true;
        out.push_str("container: Ogg\n");
    } else if head.starts_with(b"FORM") {
        other = true;
        out.push_str("container: AIFF\n");
    }
    fn parse_frame(h: &[u8]) -> Option<(u8, u8, u16, u32, usize)> {
        const BR: [[u16; 15]; 3] = [
            [
                0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
            ],
            [
                0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
            ],
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ],
        ];
        if h.len() < 4 || h[0] != 0xff || h[1] & 0xe0 != 0xe0 {
            return None;
        }
        let ver = (h[1] >> 3) & 3;
        let layer = (h[1] >> 1) & 3;
        let bri = (h[2] >> 4) as usize;
        let sri = (h[2] >> 2) & 3;
        if ver == 1 || layer == 0 || bri == 0 || bri == 15 || sri == 3 {
            return None;
        }
        let kbps = if ver == 3 {
            BR[3 - layer as usize][bri]
        } else if layer == 3 {
            [0, 32, 48, 56, 64, 80, 96, 112, 144, 160, 176, 192, 224, 256][bri.min(13)]
        } else {
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bri]
        };
        let base = [44100u32, 48000, 32000][sri as usize];
        let sr = match ver {
            3 => base,
            2 => base / 2,
            _ => base / 4,
        };
        let pad = ((h[2] >> 1) & 1) as usize;
        let kb = kbps as usize * 1000;
        let frame_len = match layer {
            3 => (12 * kb / sr as usize + pad) * 4,
            2 => 144 * kb / sr as usize + pad,
            _ if ver == 3 => 144 * kb / sr as usize + pad,
            _ => 72 * kb / sr as usize + pad,
        };
        Some((ver, layer, kbps, sr, frame_len))
    }
    let mut found = None;
    let mut i = if other {
        head.len()
    } else {
        audio_start.min(head.len())
    };
    while i + 4 <= head.len() {
        if let Some((ver, layer, kbps, sr, len1)) = parse_frame(&head[i..]) {
            let ok = parse_frame(&head[(i + len1).min(head.len())..])
                .filter(|(v, l, _, s, _)| *v == ver && *l == layer && *s == sr)
                .and_then(|(_, _, _, _, len2)| {
                    parse_frame(&head[(i + len1 + len2).min(head.len())..])
                })
                .filter(|(v, l, _, s, _)| *v == ver && *l == layer && *s == sr)
                .is_some();
            if ok {
                let vname = ["MPEG-2.5", "?", "MPEG-2", "MPEG-1"][ver as usize];
                let lname = ["?", "III", "II", "I"][layer as usize];
                found = Some((i, vname, lname, kbps, sr));
                break;
            }
        }
        i += 1;
    }
    match found {
        _ if other => {}
        Some((off, v, l, kbps, sr)) => out.push_str(&format!(
            "first MPEG frame: offset {} ({} bytes past expected audio start): {} Layer {}, {} kbps, {} Hz\n",
            off,
            off.saturating_sub(audio_start),
            v,
            l,
            kbps,
            sr
        )),
        None => out.push_str(&format!(
            "no MPEG frame header found in the first {} bytes\n",
            head.len()
        )),
    }
    let len = f.seek(SeekFrom::End(0))?;
    if len >= 128 {
        f.seek(SeekFrom::End(-128))?;
        let mut tail = [0u8; 3];
        f.read_exact(&mut tail)?;
        if &tail == b"TAG" {
            out.push_str("ID3v1 tag present at end\n");
        }
    }
    Ok(out)
}
impl Audio {
    pub fn new(fx: FxParams) -> Result<Self> {
        let mut stream = open_output().context("opening default audio output")?;
        stream.log_on_drop(false);
        let player = Player::connect_new(stream.mixer());
        if std::env::var_os("VINYL_MUTE").is_some() {
            player.set_volume(0.0);
        }
        Ok(Self {
            _stream: stream,
            player,
            fx: Arc::new(Mutex::new(fx)),
            tap: Tap::new(),
        })
    }
    pub fn load(&mut self, path: &Path, speed: f32) -> Result<()> {
        let source = open_decoder(path)?;
        let source = Fx::new(source, self.fx.clone(), Some(self.tap.clone()));
        self.player.clear();
        self.player.set_speed(speed);
        self.player.append(source);
        self.player.play();
        Ok(())
    }
    pub fn tap(&self) -> &SharedTap {
        &self.tap
    }
    pub fn set_fx(&self, p: FxParams) {
        if let Ok(mut g) = self.fx.lock() {
            *g = p;
        }
    }
    pub fn toggle_pause(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }
    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }
    pub fn pause(&self) {
        self.player.pause();
    }
    pub fn resume(&self) {
        self.player.play();
    }
    pub fn finished(&self) -> bool {
        self.player.empty()
    }
    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }
    pub fn set_speed(&self, speed: f32) {
        self.player.set_speed(speed);
    }
    pub fn seek_by(&self, delta_secs: f64, total_secs: f64) {
        let cur = self.position().as_secs_f64();
        let mut target = cur + delta_secs;
        if target < 0.0 {
            target = 0.0;
        }
        if total_secs > 0.0 && target > total_secs - 0.5 {
            target = (total_secs - 0.5).max(0.0);
        }
        let _ = self.player.try_seek(Duration::from_secs_f64(target));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;
    use std::io::Write;
    use std::time::Instant;
    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vinyl-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }
    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = temp(name);
        std::fs::File::create(&p).unwrap().write_all(bytes).unwrap();
        p
    }
    fn id3v2(version: u8, body_len: usize, footer: bool) -> Vec<u8> {
        let mut v = b"ID3".to_vec();
        v.push(version);
        v.push(0);
        v.push(if footer { 0x10 } else { 0 });
        v.extend([
            ((body_len >> 21) & 0x7f) as u8,
            ((body_len >> 14) & 0x7f) as u8,
            ((body_len >> 7) & 0x7f) as u8,
            (body_len & 0x7f) as u8,
        ]);
        v.extend(std::iter::repeat_n(0u8, body_len));
        if footer {
            v.extend(std::iter::repeat_n(0u8, 10));
        }
        v
    }
    fn pcm_wav(sr: u32, secs: f32) -> Vec<u8> {
        let frames = (sr as f32 * secs) as usize;
        let mut data = Vec::with_capacity(frames * 4);
        for i in 0..frames {
            let v = ((i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sr as f32).sin() * 12000.0)
                as i16;
            data.extend(v.to_le_bytes());
            data.extend((v / 2).to_le_bytes());
        }
        wav_with_fmt(0x0001, sr, &data, true)
    }
    fn wav_with_fmt(tag: u16, sr: u32, payload: &[u8], include_fact: bool) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend(tag.to_le_bytes());
        fmt.extend(2u16.to_le_bytes());
        fmt.extend(sr.to_le_bytes());
        fmt.extend((sr * 4).to_le_bytes());
        fmt.extend(4u16.to_le_bytes());
        fmt.extend(16u16.to_le_bytes());
        if tag == 0x55 {
            fmt.extend(12u16.to_le_bytes());
            fmt.extend(1u16.to_le_bytes());
            fmt.extend(0u32.to_le_bytes());
            fmt.extend(1u16.to_le_bytes());
            fmt.extend(1u16.to_le_bytes());
            fmt.extend(1393u16.to_le_bytes());
        }
        let mut body = b"WAVE".to_vec();
        body.extend(b"fmt ");
        body.extend((fmt.len() as u32).to_le_bytes());
        body.extend(&fmt);
        if include_fact {
            body.extend(b"fact");
            body.extend(4u32.to_le_bytes());
            body.extend(0u32.to_le_bytes());
        }
        body.extend(b"data");
        body.extend((payload.len() as u32).to_le_bytes());
        body.extend(payload);
        let mut v = b"RIFF".to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        v
    }
    fn fake_mp3_frames(n: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for _ in 0..n {
            v.extend([0xff, 0xfb, 0x90, 0x00]);
            v.extend(std::iter::repeat_n(0x11u8, 417 - 4));
        }
        v
    }
    #[test]
    fn file_slice_confines_reads_and_seeks() {
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let p = write("slice.bin", &bytes);
        let mut s = FileSlice::new(File::open(&p).unwrap(), 100, 300).unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        assert_eq!(all, &bytes[100..400]);
        assert_eq!(s.seek(SeekFrom::Start(290)).unwrap(), 290);
        let mut tail = Vec::new();
        s.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &bytes[390..400]);
        let mut none = [0u8; 4];
        assert_eq!(s.read(&mut none).unwrap(), 0);
        assert_eq!(s.seek(SeekFrom::End(-5)).unwrap(), 295);
        assert_eq!(s.seek(SeekFrom::Current(-10)).unwrap(), 285);
        let mut buf = [0u8; 2];
        s.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [bytes[385], bytes[386]]);
        assert!(s.seek(SeekFrom::Current(-1000)).is_err());
        assert_eq!(s.seek(SeekFrom::Start(10_000)).unwrap(), 10_000);
        assert_eq!(s.read(&mut buf).unwrap(), 0);
    }
    #[test]
    fn locate_audio_leaves_plain_files_alone() {
        let frames = fake_mp3_frames(5);
        let p = write("plain.mp3", &frames);
        let (_, start, len, note) = locate_audio(&p).unwrap();
        assert_eq!((start, len), (0, frames.len() as u64));
        assert!(note.is_none());
        let mut tagged = id3v2(3, 500, false);
        tagged.extend(&frames);
        let p = write("tagged.mp3", &tagged);
        let (_, start, len, note) = locate_audio(&p).unwrap();
        assert_eq!((start, len), (0, tagged.len() as u64));
        assert!(note.is_none(), "ID3 alone must not trigger unwrapping");
    }
    #[test]
    fn locate_audio_unwraps_mp3_inside_wav_after_id3() {
        let frames = fake_mp3_frames(8);
        let riff = wav_with_fmt(0x55, 44_100, &frames, true);
        let mut file = id3v2(2, 10_330, false);
        let tag_len = file.len();
        file.extend(&riff);
        let p = write("wrapped.mp3", &file);
        let (_, start, len, note) = locate_audio(&p).unwrap();
        let expected_start = tag_len + riff.len() - frames.len();
        assert_eq!(start, expected_start as u64);
        assert_eq!(len, frames.len() as u64);
        assert!(note.unwrap().contains("0x55"));
        let mut s = FileSlice::new(File::open(&p).unwrap(), start, len).unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).unwrap();
        assert_eq!(got, frames);
    }
    #[test]
    fn locate_audio_handles_footer_flag_and_zero_data_size() {
        let frames = fake_mp3_frames(3);
        let mut riff = wav_with_fmt(0x50, 44_100, &frames, false);
        let pos = riff.len() - frames.len() - 4;
        riff[pos..pos + 4].copy_from_slice(&0u32.to_le_bytes());
        let mut file = id3v2(4, 64, true);
        file.extend(&riff);
        let p = write("zero-size.mp3", &file);
        let (_, start, len, note) = locate_audio(&p).unwrap();
        assert_eq!(start as usize, file.len() - frames.len());
        assert_eq!(len, frames.len() as u64);
        assert!(note.is_some());
    }
    #[test]
    fn locate_audio_keeps_pcm_wav_whole() {
        let wav = pcm_wav(8000, 0.1);
        let p = write("pcm.wav", &wav);
        let (_, start, len, note) = locate_audio(&p).unwrap();
        assert_eq!((start, len), (0, wav.len() as u64));
        assert!(note.is_none());
    }
    #[test]
    fn decoder_reads_and_seeks_through_slice() {
        let sr = 22_050;
        let p = write("sine.wav", &pcm_wav(sr, 2.0));
        let mut dec = open_decoder(&p).unwrap();
        assert_eq!(dec.sample_rate().get(), sr);
        assert_eq!(dec.channels().get(), 2);
        let total = dec.by_ref().count();
        assert_eq!(total, sr as usize * 2 * 2);
        let mut dec = open_decoder(&p).unwrap();
        dec.try_seek(Duration::from_millis(1500)).unwrap();
        let rest = dec.count();
        let expected = (sr as usize / 2) * 2;
        assert!(
            (rest as i64 - expected as i64).abs() <= 2 * 2 * 1024,
            "after seek got {rest}, expected about {expected}"
        );
    }
    #[test]
    fn decoder_accepts_id3_before_pcm_wav() {
        let mut bytes = id3v2(3, 2048, false);
        bytes.extend(pcm_wav(8000, 0.5));
        let p = write("id3-pcm.wav", &bytes);
        let dec = open_decoder(&p).unwrap();
        assert_eq!(dec.sample_rate().get(), 8000);
        assert_eq!(dec.count(), 8000);
    }
    #[test]
    fn normalisation_gain_targets_rms_and_ignores_silence() {
        // -18 dBFS RMS sine: amplitude 0.178; a track already at target needs 0 dB.
        let amp = 10f32.powf(-18.0 / 20.0) * 2f32.sqrt();
        let tone: Vec<f32> = (0..44_100 * 4)
            .map(|i| (i as f32 * 0.05).sin() * amp)
            .collect();
        let g = gain_for(block_levels(tone.iter().copied(), 17_640));
        assert!(g.abs() < 0.2, "at target gave {g:+.2}");
        // Half the amplitude (-6 dB) wants +6 dB, silence around it ignored.
        let mut padded = vec![0.0f32; 44_100 * 4];
        padded.extend(tone.iter().map(|v| v * 0.5));
        padded.extend(vec![0.0f32; 44_100 * 4]);
        let g = gain_for(block_levels(padded.iter().copied(), 17_640));
        assert!((g - 6.0).abs() < 0.2, "quiet gave {g:+.2}");
        // Very loud is cut, very quiet is boosted, both within range.
        let g = gain_for(block_levels(tone.iter().map(|v| v * 5.0), 17_640));
        assert!((g + 14.0).abs() < 0.2, "loud gave {g:+.2}");
        let g = gain_for(block_levels(tone.iter().map(|v| v * 0.01), 17_640));
        assert_eq!(g, NORMALIZE_RANGE_DB.1);
        // Below the gate everything counts as silence: leave it alone.
        let g = gain_for(block_levels(tone.iter().map(|v| v * 0.001), 17_640));
        assert_eq!(g, 0.0);
        assert_eq!(
            gain_for(block_levels(std::iter::repeat_n(0.0, 100_000), 17_640)),
            0.0
        );
    }
    #[test]
    fn open_decoder_reports_missing_file() {
        let err = match open_decoder(&temp("does-not-exist.mp3")) {
            Ok(_) => panic!("missing file should not open"),
            Err(e) => e,
        };
        assert!(format!("{err:#}").contains("opening"));
    }
    #[test]
    fn probe_describes_wrapper_and_decodes_pcm() {
        let p = write("probe.wav", &pcm_wav(8000, 0.25));
        let report = probe(&p, FxParams::default()).unwrap();
        assert!(report.contains("container: RIFF/WAVE"));
        assert!(report.contains("decoder: 8000 Hz, 2 ch"));
        assert!(report.contains("through fx:"));
        let frames = fake_mp3_frames(6);
        let mut wrapped = id3v2(2, 12, false);
        wrapped.extend(wav_with_fmt(0x55, 44_100, &frames, true));
        let p = write("probe-wrapped.mp3", &wrapped);
        let report = probe(&p, FxParams::default()).unwrap();
        assert!(report.contains("ID3v2.2 tag: 12 bytes"));
        assert!(report.contains("after tag: 52 49 46 46"));
        assert!(report.contains("MPEG-1 Layer III, 128 kbps, 44100 Hz"));
        assert!(report.contains("unwrap: RIFF/WAVE wrapper"));
        assert!(report.contains("decoder:"));
    }
    #[test]
    fn describe_header_ignores_random_sync_bytes_without_consistent_frames() {
        let mut junk: Vec<u8> = Vec::new();
        let mut x = 0x9e37_79b9u32;
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            junk.push((x >> 24) as u8);
        }
        let p = write("junk.bin", &junk);
        let report = describe_header(&p).unwrap();
        assert!(
            report.contains("no MPEG frame header found"),
            "report was: {report}"
        );
    }
    #[test]
    #[ignore]
    fn soak_decoder_seeks_and_streams_long_file() {
        let sr = 44_100;
        let secs = 600.0f64;
        let p = write("soak.wav", &pcm_wav(sr, secs as f32));
        let t = Instant::now();
        let mut dec = open_decoder(&p).unwrap();
        let mut n = 0usize;
        let mut peak = 0f32;
        for s in dec.by_ref() {
            n += 1;
            assert!(s.is_finite());
            peak = peak.max(s.abs());
        }
        let stream = t.elapsed();
        assert_eq!(n, sr as usize * 2 * secs as usize);
        assert!(peak > 0.3 && peak < 0.4, "peak {peak}");
        let mut rng = 0xdead_beefu64;
        let t = Instant::now();
        for _ in 0..1000 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let ms = rng % (secs as u64 * 1000);
            dec.try_seek(Duration::from_millis(ms)).unwrap();
            let got = dec.by_ref().take(4096).count();
            assert!(
                got > 0 || ms > (secs as u64 - 1) * 1000,
                "seek to {ms} ms gave nothing"
            );
        }
        let seeks = t.elapsed();
        eprintln!(
            "soak decoder: {secs}s streamed in {:.2}s ({:.0}x realtime); 1000 seeks in {:.2}s",
            stream.as_secs_f64(),
            secs / stream.as_secs_f64(),
            seeks.as_secs_f64()
        );
        assert!(stream.as_secs_f64() < secs / 5.0, "slower than 5x realtime");
    }
}
