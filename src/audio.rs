use crate::fx::{Fx, FxParams, SharedParams};
use crate::spectrum::{SharedTap, Tap};
use anyhow::{Context, Result};
use rodio::cpal::BufferSize;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
fn open_output() -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
    let first = DeviceSinkBuilder::from_default_device()
        .and_then(|b| b.with_buffer_size(BufferSize::Default).open_stream());
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
/// A window onto a file: reads and seeks are confined to `start..start+len`.
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
/// Where the decodable audio lives inside `path`, plus an optional note about any
/// wrapper that was skipped. Handles the old iTunes/QuickTime habit of writing MP3
/// frames inside a RIFF/WAVE container (format tag 0x55), which symphonia's WAV
/// reader rejects: in that case the slice covers just the `data` chunk, so the MP3
/// reader sees bare frames.
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
    if off < file_len && file.read(&mut riff)? == 12 && &riff[0..4] == b"RIFF" && &riff[8..12] == b"WAVE"
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
            let len = if size == 0 || size > avail { avail } else { size };
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
        .with_data(BufReader::new(slice))
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
/// Diagnostic: open `path` like playback does, pull a few seconds of audio through
/// the FX chain and describe what came out. Used by `vinyl --probe <file>`.
pub fn probe(path: &Path, fx: FxParams) -> Result<String> {
    use rodio::Source;
    let mut out = String::new();
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    out.push_str(&format!("file: {} ({} bytes)\n", path.display(), meta.len()));
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
/// Describe the raw bytes at the start of a file: container magic, ID3v2 tag size and the
/// position/shape of the first plausible MPEG audio frame header.
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
    // Scan for the first MPEG audio frame header that is followed by two more
    // consistent frames (so random PCM data does not masquerade as a frame).
    fn parse_frame(h: &[u8]) -> Option<(u8, u8, u16, u32, usize)> {
        const BR: [[u16; 15]; 3] = [
            [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
            [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
            [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
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
    let mut i = if other { head.len() } else { audio_start.min(head.len()) };
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
