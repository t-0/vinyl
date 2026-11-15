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
        let v =
            ((i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sr as f32).sin() * 12000.0) as i16;
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
