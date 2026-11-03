use crate::spectrum::SharedTap;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Off,
    Vinyl,
    ReelToReel,
    Cassette,
}
impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Off, Mode::Vinyl, Mode::ReelToReel, Mode::Cassette];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Off => "Off",
            Mode::Vinyl => "Vinyl",
            Mode::ReelToReel => "Reel-to-reel",
            Mode::Cassette => "Cassette",
        }
    }
    pub fn is_tape(self) -> bool {
        matches!(self, Mode::ReelToReel | Mode::Cassette)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum EqMode {
    #[default]
    Off,
    Three,
    Seven,
}
impl EqMode {
    pub const ALL: [EqMode; 3] = [EqMode::Off, EqMode::Three, EqMode::Seven];
    pub fn label(self) -> &'static str {
        match self {
            EqMode::Off => "Off",
            EqMode::Three => "3-band",
            EqMode::Seven => "7-band",
        }
    }
    pub fn bands(self) -> usize {
        match self {
            EqMode::Off => 0,
            EqMode::Three => 3,
            EqMode::Seven => 7,
        }
    }
}
pub const EQ_RANGE_DB: i8 = 12;
pub const THREE_BANDS: [(&str, f32); 3] = [("Low", 200.0), ("Mid", 1000.0), ("High", 4000.0)];
pub const SEVEN_BANDS: [f32; 7] = [60.0, 150.0, 400.0, 1000.0, 2500.0, 6000.0, 15000.0];
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub struct EqParams {
    pub mode: EqMode,
    pub three: [i8; 3],
    pub seven: [i8; 7],
}
impl EqParams {
    pub fn gain(&self, band: usize) -> i8 {
        match self.mode {
            EqMode::Off => 0,
            EqMode::Three => self.three.get(band).copied().unwrap_or(0),
            EqMode::Seven => self.seven.get(band).copied().unwrap_or(0),
        }
    }
    pub fn gain_mut(&mut self, band: usize) -> Option<&mut i8> {
        match self.mode {
            EqMode::Off => None,
            EqMode::Three => self.three.get_mut(band),
            EqMode::Seven => self.seven.get_mut(band),
        }
    }
    pub fn band_label(&self, band: usize) -> String {
        match self.mode {
            EqMode::Off => String::new(),
            EqMode::Three => format!(
                "{:<5}{:>5} Hz",
                THREE_BANDS[band].0, THREE_BANDS[band].1 as u32
            ),
            EqMode::Seven => {
                let f = SEVEN_BANDS[band];
                if f >= 1000.0 {
                    format!("{:>7.1} kHz", f / 1000.0)
                } else {
                    format!("{:>8} Hz", f as u32)
                }
            }
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum StereoMode {
    #[default]
    Normal,
    Mono,
    Width,
}
impl StereoMode {
    pub const ALL: [StereoMode; 3] = [StereoMode::Normal, StereoMode::Mono, StereoMode::Width];
    pub fn label(self) -> &'static str {
        match self {
            StereoMode::Normal => "Normal",
            StereoMode::Mono => "Mono",
            StereoMode::Width => "Width",
        }
    }
}
pub const WIDTH_MAX: u8 = 200;
fn default_width() -> u8 {
    150
}
fn default_hiss() -> u8 {
    50
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FxParams {
    pub mode: Mode,
    pub scratch: u8,
    pub distortion: u8,
    pub age: u8,
    pub saturation: u8,
    pub wow: u8,
    pub flutter: u8,
    #[serde(default = "default_hiss")]
    pub hiss: u8,
    #[serde(default)]
    pub eq: EqParams,
    #[serde(default)]
    pub loudness: bool,
    #[serde(default)]
    pub stereo: StereoMode,
    #[serde(default = "default_width")]
    pub width: u8,
}
impl Default for FxParams {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            scratch: 30,
            distortion: 15,
            age: 25,
            saturation: 35,
            wow: 20,
            flutter: 15,
            hiss: default_hiss(),
            eq: EqParams::default(),
            loudness: false,
            stereo: StereoMode::Normal,
            width: default_width(),
        }
    }
}
pub type SharedParams = Arc<Mutex<FxParams>>;
struct Rng(u32);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
    fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}
impl Biquad {
    fn set_coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    fn set_peaking(&mut self, sr: f32, fc: f32, q: f32, gain_db: f32) {
        let fc = fc.clamp(20.0, sr * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        self.set_coeffs(
            1.0 + alpha * a,
            -2.0 * cw,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cw,
            1.0 - alpha / a,
        );
    }
    fn set_lowshelf(&mut self, sr: f32, fc: f32, gain_db: f32) {
        let fc = fc.clamp(20.0, sr * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / 2.0 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * alpha;
        self.set_coeffs(
            a * ((a + 1.0) - (a - 1.0) * cw + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
            a * ((a + 1.0) - (a - 1.0) * cw - sa),
            (a + 1.0) + (a - 1.0) * cw + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cw),
            (a + 1.0) + (a - 1.0) * cw - sa,
        );
    }
    fn set_highshelf(&mut self, sr: f32, fc: f32, gain_db: f32) {
        let fc = fc.clamp(20.0, sr * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / 2.0 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * alpha;
        self.set_coeffs(
            a * ((a + 1.0) + (a - 1.0) * cw + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
            a * ((a + 1.0) + (a - 1.0) * cw - sa),
            (a + 1.0) - (a - 1.0) * cw + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cw),
            (a + 1.0) - (a - 1.0) * cw - sa,
        );
    }
    fn set_lowpass(&mut self, sr: f32, fc: f32) {
        let fc = fc.clamp(200.0, sr * 0.45);
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - cw) / 2.0 / a0;
        self.b1 = (1.0 - cw) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cw / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
struct Channel {
    lowpass: Biquad,
    delay: Vec<f32>,
    wpos: usize,
    eq: [Biquad; 7],
    loud: [Biquad; 2],
}
#[inline]
fn soft_limit(y: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = y.abs();
    if a <= KNEE {
        y
    } else {
        y.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}
#[derive(Clone, Copy, Default)]
struct Derived {
    mode: Mode,
    drive: f32,
    bias: f32,
    makeup: f32,
    hiss: f32,
    wow_depth: f32,
    flutter_depth: f32,
    lfo_scale: f32,
    pop_prob: f32,
    pop_level: f32,
    dropout_prob: f32,
    cutoff: f32,
    eq_bands: usize,
    loudness: bool,
    side_gain: Option<f32>,
}
pub struct Fx<S: Source> {
    inner: S,
    params: SharedParams,
    d: Derived,
    channels: usize,
    sr: f32,
    chans: Vec<Channel>,
    out: Vec<f32>,
    out_pos: usize,
    tap: Option<SharedTap>,
    tap_pending: Vec<f32>,
    frames: u64,
    rng: Rng,
    frame_delay: f32,
    frame_pop: f32,
    frame_gain: f32,
    pop_env: f32,
    pop_decay: f32,
    lfo: [f32; 4],
    dropout_left: u32,
    dropout_len: u32,
    dropout_depth: f32,
}
const REFRESH_FRAMES: u64 = 256;
const BASE_DELAY_SECS: f32 = 0.016;
impl<S: Source> Fx<S> {
    pub fn new(inner: S, params: SharedParams, tap: Option<SharedTap>) -> Self {
        let channels = inner.channels().get() as usize;
        let sr = inner.sample_rate().get() as f32;
        let delay_len = (sr * 0.05) as usize + 16;
        let chans = (0..channels)
            .map(|_| Channel {
                lowpass: Biquad::default(),
                delay: vec![0.0; delay_len],
                wpos: 0,
                eq: [Biquad::default(); 7],
                loud: [Biquad::default(); 2],
            })
            .collect();
        let mut fx = Self {
            inner,
            params,
            d: Derived::default(),
            channels,
            sr,
            chans,
            out: Vec::with_capacity(channels),
            out_pos: 0,
            tap,
            tap_pending: Vec::with_capacity(REFRESH_FRAMES as usize),
            frames: 0,
            rng: Rng(0x9E37_79B9),
            frame_delay: BASE_DELAY_SECS,
            frame_pop: 0.0,
            frame_gain: 1.0,
            pop_env: 0.0,
            pop_decay: (-1.0 / (sr * 0.0012)).exp(),
            lfo: [0.0; 4],
            dropout_left: 0,
            dropout_len: 1,
            dropout_depth: 0.0,
        };
        fx.refresh();
        fx
    }
    fn flush_tap(&mut self) {
        if let Some(tap) = &self.tap
            && !self.tap_pending.is_empty()
            && let Ok(mut t) = tap.try_lock()
        {
            t.sample_rate = self.sr;
            t.push(&self.tap_pending);
            self.tap_pending.clear();
        }
        if self.tap_pending.len() > 4096 {
            self.tap_pending.clear();
        }
    }
    fn refresh(&mut self) {
        let p = match self.params.try_lock() {
            Ok(p) => *p,
            Err(_) => return,
        };
        let pct = |v: u8| v.min(100) as f32 / 100.0;
        let mut d = Derived {
            mode: p.mode,
            makeup: 1.0,
            lfo_scale: 1.0,
            cutoff: 20_000.0,
            ..Default::default()
        };
        match p.mode {
            Mode::Off => {}
            Mode::Vinyl => {
                let dist = pct(p.distortion);
                let scratch = pct(p.scratch);
                d.drive = 1.0 + 6.0 * dist;
                d.bias = 0.1 * dist;
                d.makeup = 1.0 + 1.2 * dist;
                d.hiss = 0.0025 * scratch;
                d.pop_prob = 0.00025 * scratch.powf(1.5);
                d.pop_level = 0.25 + 0.75 * scratch;
                d.cutoff = 16_000.0;
            }
            Mode::ReelToReel | Mode::Cassette => {
                let cassette = p.mode == Mode::Cassette;
                let age = pct(p.age);
                let sat = pct(p.saturation) + if cassette { 0.1 } else { 0.0 };
                d.drive = 1.0 + 5.0 * sat;
                d.makeup = 1.0 / d.drive.powf(0.65);
                // Hiss 50 matches the old fixed level; 0 is silent, 100 is double.
                let base_hiss = if cassette { 0.0035 } else { 0.0010 };
                d.hiss = base_hiss * 2.0 * pct(p.hiss) * (1.0 + 3.0 * age);
                let wf_scale = if cassette { 1.5 } else { 0.6 };
                d.wow_depth = (0.005 * pct(p.wow) + 0.0008 * age) * wf_scale;
                d.flutter_depth = 0.00024 * pct(p.flutter) * wf_scale;
                d.lfo_scale = if cassette { 1.15 } else { 0.85 };
                d.dropout_prob = 6.0e-6 * age * age;
                let base_cut = if cassette { 12_000.0 } else { 18_000.0 };
                d.cutoff = base_cut * (1.0 - 0.6 * age);
            }
        }
        let rate_scale = 44_100.0 / self.sr;
        d.pop_prob *= rate_scale;
        d.dropout_prob *= rate_scale;
        d.eq_bands = p.eq.mode.bands();
        d.loudness = p.loudness;
        d.side_gain = match p.stereo {
            StereoMode::Normal => None,
            StereoMode::Mono => Some(0.0),
            StereoMode::Width => Some(p.width.min(WIDTH_MAX) as f32 / 100.0),
        };
        let sr = self.sr;
        for c in &mut self.chans {
            c.lowpass.set_lowpass(sr, d.cutoff);
            match p.eq.mode {
                EqMode::Off => {}
                EqMode::Three => {
                    c.eq[0].set_lowshelf(sr, THREE_BANDS[0].1, p.eq.three[0] as f32);
                    c.eq[1].set_peaking(sr, THREE_BANDS[1].1, 0.7, p.eq.three[1] as f32);
                    c.eq[2].set_highshelf(sr, THREE_BANDS[2].1, p.eq.three[2] as f32);
                }
                EqMode::Seven => {
                    for (i, f) in SEVEN_BANDS.iter().enumerate() {
                        c.eq[i].set_peaking(sr, *f, 1.1, p.eq.seven[i] as f32);
                    }
                }
            }
            if p.loudness {
                c.loud[0].set_lowshelf(sr, 100.0, 7.0);
                c.loud[1].set_highshelf(sr, 10_000.0, 2.5);
            }
        }
        self.d = d;
    }
    fn advance_frame(&mut self) {
        let d = self.d;
        let dt = 1.0 / self.sr;
        let two_pi = 2.0 * std::f32::consts::PI;
        if d.mode.is_tape() {
            let f = [0.55, 0.93, 6.8, 11.3];
            for (phase, freq) in self.lfo.iter_mut().zip(f) {
                *phase += two_pi * freq * d.lfo_scale * dt;
                if *phase > two_pi {
                    *phase -= two_pi;
                }
            }
            let wow = d.wow_depth * (0.6 * self.lfo[0].sin() + 0.4 * self.lfo[1].sin());
            let fl = d.flutter_depth * (0.5 * self.lfo[2].sin() + 0.5 * self.lfo[3].sin());
            self.frame_delay = BASE_DELAY_SECS + wow + fl;
            if self.dropout_left > 0 {
                self.dropout_left -= 1;
                let t = 1.0 - self.dropout_left as f32 / self.dropout_len as f32;
                self.frame_gain = 1.0 - self.dropout_depth * (std::f32::consts::PI * t).sin();
            } else {
                self.frame_gain = 1.0;
                if d.dropout_prob > 0.0 && self.rng.unit() < d.dropout_prob {
                    self.dropout_len = (self.sr * (0.015 + 0.045 * self.rng.unit())) as u32 + 1;
                    self.dropout_left = self.dropout_len;
                    self.dropout_depth = 0.4 + 0.55 * self.rng.unit();
                }
            }
        } else {
            self.frame_delay = BASE_DELAY_SECS;
            self.frame_gain = 1.0;
        }
        if d.mode == Mode::Vinyl {
            if d.pop_prob > 0.0 && self.rng.unit() < d.pop_prob {
                let amp = (0.05 + 0.45 * self.rng.unit().powi(2)) * d.pop_level;
                self.pop_env = if self.rng.unit() < 0.5 { amp } else { -amp };
            }
            self.frame_pop = self.pop_env;
            self.pop_env *= self.pop_decay;
        } else {
            self.frame_pop = 0.0;
        }
    }
    #[inline]
    fn process(&mut self, x: Sample, ch: usize) -> Sample {
        let d = self.d;
        let shape = d.eq_bands > 0 || d.loudness;
        if d.mode == Mode::Off && !shape {
            return x;
        }
        let mut y = x;
        let sr = self.sr;
        let c = &mut self.chans[ch];
        if d.mode == Mode::Off {
        } else if d.mode.is_tape() {
            let len = c.delay.len();
            c.delay[c.wpos] = y;
            let delay_samples = (self.frame_delay * sr).clamp(1.0, (len - 3) as f32);
            let rp = c.wpos as f32 + len as f32 - delay_samples;
            let i0 = rp.floor();
            let frac = rp - i0;
            let i0 = i0 as usize % len;
            let i1 = (i0 + 1) % len;
            y = c.delay[i0] * (1.0 - frac) + c.delay[i1] * frac;
            c.wpos = (c.wpos + 1) % len;
            y = (y * d.drive).tanh() * d.makeup;
        } else {
            let offset = (d.bias * d.drive).tanh();
            y = (((y + d.bias) * d.drive).tanh() - offset) / d.drive * d.makeup;
        }
        if d.mode != Mode::Off {
            if d.hiss > 0.0 {
                y += self.rng.bipolar() * d.hiss;
            }
            y = c.lowpass.process(y);
            y += self.frame_pop;
            y *= self.frame_gain;
        }
        if shape {
            for f in c.eq.iter_mut().take(d.eq_bands) {
                y = f.process(y);
            }
            if d.loudness {
                let bass = c.loud[0].process(y * 0.75);
                y = c.loud[1].process(bass);
            }
            y = soft_limit(y);
        }
        y
    }
}
impl<S: Source> Iterator for Fx<S> {
    type Item = Sample;
    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.out_pos < self.out.len() {
            let y = self.out[self.out_pos];
            self.out_pos += 1;
            return Some(y);
        }
        self.out.clear();
        self.out_pos = 0;
        for _ in 0..self.channels {
            match self.inner.next() {
                Some(x) => self.out.push(x),
                None => break,
            }
        }
        if self.out.is_empty() {
            return None;
        }
        if self.frames.is_multiple_of(REFRESH_FRAMES) {
            self.refresh();
            self.flush_tap();
        }
        self.frames += 1;
        self.advance_frame();
        for ch in 0..self.out.len() {
            let x = self.out[ch];
            self.out[ch] = self.process(x, ch);
        }
        if let Some(sg) = self.d.side_gain
            && self.out.len() == 2
        {
            let (l, r) = (self.out[0], self.out[1]);
            let mid = 0.5 * (l + r);
            let side = 0.5 * (l - r) * sg;
            self.out[0] = soft_limit(mid + side);
            self.out[1] = soft_limit(mid - side);
        }
        if self.tap.is_some() {
            let mono = self.out.iter().sum::<f32>() / self.out.len() as f32;
            self.tap_pending.push(mono);
        }
        self.out_pos = 1;
        Some(self.out[0])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl<S: Source> Source for Fx<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;
    fn sine(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 44_100.0).sin() * 0.5)
            .collect()
    }
    fn run(params: FxParams) -> Vec<f32> {
        let data = sine(44_100);
        let src = SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(44_100).unwrap(),
            data,
        );
        let shared = Arc::new(Mutex::new(params));
        Fx::new(src, shared, None).collect()
    }
    #[test]
    fn off_is_identity() {
        let out = run(FxParams {
            mode: Mode::Off,
            ..Default::default()
        });
        assert_eq!(out, sine(44_100));
    }
    #[test]
    fn modes_stay_finite_and_bounded() {
        for mode in [Mode::Vinyl, Mode::ReelToReel, Mode::Cassette] {
            let p = FxParams {
                mode,
                scratch: 100,
                distortion: 100,
                age: 100,
                saturation: 100,
                wow: 100,
                flutter: 100,
                ..Default::default()
            };
            let out = run(p);
            assert_eq!(out.len(), 44_100);
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak.is_finite() && peak < 1.5, "{mode:?} peak {peak}");
            let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
            assert!(rms > 0.05, "{mode:?} rms {rms} too quiet");
        }
    }
    #[test]
    fn eq_and_loudness_stay_bounded_and_do_something() {
        let clean = sine(44_100);
        let mut p = FxParams {
            loudness: true,
            ..Default::default()
        };
        p.eq.mode = EqMode::Seven;
        p.eq.seven = [12; 7];
        let out = run(p);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak.is_finite() && peak <= 1.0, "peak {peak}");
        let diff: f32 = out
            .iter()
            .zip(&clean)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / clean.len() as f32;
        assert!(diff > 0.01, "eq left the signal unchanged");
        let mut flat = FxParams::default();
        flat.eq.mode = EqMode::Three;
        let out = run(flat);
        let diff: f32 = out
            .iter()
            .zip(&clean)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / clean.len() as f32;
        assert!(diff < 1e-4, "flat eq changed the signal by {diff}");
    }
    #[test]
    fn stereo_modes_fold_and_widen() {
        let n = 22_050;
        let mut data = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / 44_100.0;
            data.push((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.3);
            data.push((t * 660.0 * 2.0 * std::f32::consts::PI).sin() * 0.3);
        }
        let shared = |p: FxParams| Arc::new(Mutex::new(p));
        let side = |out: &[f32]| {
            out.chunks(2).map(|f| (f[0] - f[1]).abs()).sum::<f32>() / (out.len() / 2) as f32
        };
        let mk = || {
            SamplesBuffer::new(
                NonZero::new(2).unwrap(),
                NonZero::new(44_100).unwrap(),
                data.clone(),
            )
        };
        let normal: Vec<f32> = Fx::new(mk(), shared(FxParams::default()), None).collect();
        assert_eq!(normal, data);
        let mono: Vec<f32> = Fx::new(
            mk(),
            shared(FxParams {
                stereo: StereoMode::Mono,
                ..Default::default()
            }),
            None,
        )
        .collect();
        assert!(side(&mono) < 1e-6, "mono still has side signal");
        let wide: Vec<f32> = Fx::new(
            mk(),
            shared(FxParams {
                stereo: StereoMode::Width,
                width: 200,
                ..Default::default()
            }),
            None,
        )
        .collect();
        let (s0, s1) = (side(&data), side(&wide));
        assert!(s1 > s0 * 1.5, "width 200% did not widen: {s0} -> {s1}");
        assert!(wide.iter().all(|v| v.abs() <= 1.0));
    }
    #[test]
    fn tape_modes_alter_signal() {
        let clean = sine(44_100);
        let out = run(FxParams {
            mode: Mode::Cassette,
            ..Default::default()
        });
        let diff: f32 = out
            .iter()
            .zip(&clean)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / clean.len() as f32;
        assert!(diff > 0.001, "cassette left the signal unchanged");
    }
}
