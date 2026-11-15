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
/// Preamp gain range in dB. It sits in front of the whole chain, so a boost
/// drives the tape saturation and vinyl distortion harder and a cut backs
/// them off.
pub const PREAMP_MIN_DB: i8 = -24;
pub const PREAMP_MAX_DB: i8 = 12;
/// Where auto-normalisation tries to put a track's gated RMS level, in dBFS,
/// and how far it is allowed to move a track to get there.
pub const NORMALIZE_TARGET_DB: f32 = -18.0;
pub const NORMALIZE_RANGE_DB: (f32, f32) = (-18.0, 12.0);
/// Time constant of the ramp between preamp gains, so a measurement that
/// arrives mid-track fades in rather than steps.
const GAIN_RAMP_SECS: f32 = 0.3;
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
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
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
    /// Gain in dB applied before everything else.
    #[serde(default)]
    pub preamp: i8,
    /// Add each track's measured normalisation gain to the preamp.
    #[serde(default)]
    pub auto_normalize: bool,
    /// Normalisation gain for the current track in dB, set by the app per
    /// track; it is state, not a setting, so it is never saved.
    #[serde(skip)]
    pub track_gain: f32,
    #[serde(default)]
    pub eq: EqParams,
    #[serde(default)]
    pub loudness: bool,
    #[serde(default)]
    pub stereo: StereoMode,
    #[serde(default = "default_width")]
    pub width: u8,
}
impl FxParams {
    /// User preamp plus the track's normalisation gain when that is on, in dB.
    pub fn effective_preamp_db(&self) -> f32 {
        let user = self.preamp.clamp(PREAMP_MIN_DB, PREAMP_MAX_DB) as f32;
        if self.auto_normalize {
            user + self
                .track_gain
                .clamp(NORMALIZE_RANGE_DB.0, NORMALIZE_RANGE_DB.1)
        } else {
            user
        }
    }
    fn preamp_linear(&self) -> f32 {
        10f32.powf(self.effective_preamp_db() / 20.0)
    }
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
            preamp: 0,
            auto_normalize: false,
            track_gain: 0.0,
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
    /// Linear preamp gain; 1.0 is 0 dB.
    preamp: f32,
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
    /// Current preamp gain, ramping toward `d.preamp`.
    gain: f32,
    gain_coef: f32,
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
        let gain = params.lock().map(|p| p.preamp_linear()).unwrap_or(1.0);
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
            gain,
            gain_coef: 1.0 - (-1.0 / (sr * GAIN_RAMP_SECS)).exp(),
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
        d.preamp = p.preamp_linear();
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
        if self.gain != d.preamp {
            self.gain += (d.preamp - self.gain) * self.gain_coef;
            if (d.preamp - self.gain).abs() < 1e-4 {
                self.gain = d.preamp;
            }
        }
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
        let gain = self.gain;
        let boost = gain > 1.0;
        if d.mode == Mode::Off && !shape && gain == 1.0 {
            return x;
        }
        let mut y = x * gain;
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
        }
        if shape || boost {
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
    fn run_signal(params: FxParams, data: Vec<f32>, sr: u32) -> Vec<f32> {
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(sr).unwrap(), data);
        Fx::new(src, Arc::new(Mutex::new(params)), None).collect()
    }
    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }
    fn clean_tape(mode: Mode, hiss: u8) -> FxParams {
        FxParams {
            mode,
            age: 0,
            saturation: 0,
            wow: 0,
            flutter: 0,
            hiss,
            ..Default::default()
        }
    }
    #[test]
    fn hiss_parameter_scales_tape_noise() {
        for mode in [Mode::ReelToReel, Mode::Cassette] {
            let silence = vec![0.0f32; 44_100 * 2];
            let quiet = run_signal(clean_tape(mode, 0), silence.clone(), 44_100);
            let mid = run_signal(clean_tape(mode, 50), silence.clone(), 44_100);
            let loud = run_signal(clean_tape(mode, 100), silence, 44_100);
            let (q, m, l) = (rms(&quiet), rms(&mid), rms(&loud));
            assert!(q < 1e-6, "{mode:?}: hiss 0 should be silent, rms {q}");
            assert!(m > 1e-4, "{mode:?}: hiss 50 should be audible, rms {m}");
            let ratio = l / m;
            assert!(
                (1.6..2.4).contains(&ratio),
                "{mode:?}: hiss 100 should be about twice hiss 50, got {ratio}"
            );
        }
    }
    #[test]
    fn cassette_hisses_more_than_reel_at_same_setting() {
        let silence = vec![0.0f32; 44_100 * 2];
        let reel = rms(&run_signal(
            clean_tape(Mode::ReelToReel, 50),
            silence.clone(),
            44_100,
        ));
        let cass = rms(&run_signal(clean_tape(Mode::Cassette, 50), silence, 44_100));
        assert!(cass > reel * 2.0, "cassette {cass} vs reel {reel}");
    }
    #[test]
    fn age_still_raises_hiss() {
        let silence = vec![0.0f32; 44_100 * 2];
        let young = rms(&run_signal(
            clean_tape(Mode::ReelToReel, 50),
            silence.clone(),
            44_100,
        ));
        let old = rms(&run_signal(
            FxParams {
                age: 100,
                ..clean_tape(Mode::ReelToReel, 50)
            },
            silence,
            44_100,
        ));
        assert!(old > young * 2.0, "old {old} vs young {young}");
    }
    #[test]
    fn legacy_settings_without_hiss_load_at_default() {
        let json = r#"{"mode":"Cassette","scratch":30,"distortion":15,"age":25,"saturation":35,"wow":20,"flutter":15}"#;
        let p: FxParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.hiss, FxParams::default().hiss);
        assert_eq!(p.mode, Mode::Cassette);
        let round: FxParams = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(round, p);
    }
    #[test]
    fn vinyl_scratch_zero_is_free_of_noise_and_pops() {
        let silence = vec![0.0f32; 44_100 * 2];
        let out = run_signal(
            FxParams {
                mode: Mode::Vinyl,
                scratch: 0,
                distortion: 0,
                ..Default::default()
            },
            silence,
            44_100,
        );
        assert!(rms(&out) < 1e-6);
    }
    #[test]
    fn mono_input_is_supported() {
        let data = sine(22_050);
        let src = SamplesBuffer::new(
            NonZero::new(1).unwrap(),
            NonZero::new(22_050).unwrap(),
            data,
        );
        let p = FxParams {
            mode: Mode::Cassette,
            ..Default::default()
        };
        let out: Vec<f32> = Fx::new(src, Arc::new(Mutex::new(p)), None).collect();
        assert_eq!(out.len(), 22_050);
        assert!(out.iter().all(|v| v.is_finite()));
    }
    #[test]
    fn parameter_changes_apply_while_streaming() {
        let shared = Arc::new(Mutex::new(FxParams {
            mode: Mode::Off,
            ..Default::default()
        }));
        let data = vec![0.0f32; 44_100 * 2];
        let src = SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(44_100).unwrap(),
            data,
        );
        let mut fx = Fx::new(src, shared.clone(), None);
        let first: Vec<f32> = fx.by_ref().take(44_100).collect();
        assert!(rms(&first) < 1e-6);
        shared.lock().unwrap().mode = Mode::Cassette;
        shared.lock().unwrap().hiss = 100;
        let rest: Vec<f32> = fx.collect();
        assert!(
            rms(&rest[4096..]) > 1e-4,
            "hiss should appear after the switch"
        );
    }
    #[test]
    fn fx_chain_runs_faster_than_realtime() {
        let secs = 20;
        let data = sine(44_100 * 2 * secs);
        for mode in [Mode::Vinyl, Mode::ReelToReel, Mode::Cassette] {
            let p = FxParams {
                mode,
                eq: EqParams {
                    mode: EqMode::Seven,
                    ..Default::default()
                },
                loudness: true,
                stereo: StereoMode::Width,
                ..Default::default()
            };
            let src = SamplesBuffer::new(
                NonZero::new(2).unwrap(),
                NonZero::new(44_100).unwrap(),
                data.clone(),
            );
            let t = std::time::Instant::now();
            let n = Fx::new(src, Arc::new(Mutex::new(p)), None).fold(0usize, |n, v| {
                assert!(v.is_finite());
                n + 1
            });
            let el = t.elapsed().as_secs_f64();
            assert_eq!(n, data.len());
            eprintln!(
                "fx {mode:?}: {secs}s of audio in {el:.3}s ({:.1}x realtime)",
                secs as f64 / el
            );
            assert!(
                el < secs as f64 / 3.0,
                "{mode:?} ran at under 3x realtime: {el:.2}s"
            );
        }
    }
    #[test]
    #[ignore]
    fn soak_fx_hour_with_random_parameter_changes() {
        let sr = 44_100u32;
        let secs = 3600usize;
        let shared = Arc::new(Mutex::new(FxParams::default()));
        let block = sine(sr as usize * 2);
        let data = block.iter().copied().cycle().take(sr as usize * 2 * secs);
        struct Cycle<I: Iterator<Item = f32>>(I);
        impl<I: Iterator<Item = f32>> Iterator for Cycle<I> {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                self.0.next()
            }
        }
        impl<I: Iterator<Item = f32>> Source for Cycle<I> {
            fn current_span_len(&self) -> Option<usize> {
                None
            }
            fn channels(&self) -> ChannelCount {
                NonZero::new(2).unwrap()
            }
            fn sample_rate(&self) -> SampleRate {
                NonZero::new(44_100).unwrap()
            }
            fn total_duration(&self) -> Option<Duration> {
                None
            }
        }
        let mut fx = Fx::new(Cycle(data), shared.clone(), None);
        let mut rng = 0x5eed_1234u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let half_second = sr as usize;
        let t = std::time::Instant::now();
        let mut peak = 0f32;
        let mut total = 0usize;
        for _ in 0..secs * 2 {
            {
                let mut p = shared.lock().unwrap();
                p.mode = Mode::ALL[(next() % Mode::ALL.len() as u64) as usize];
                p.scratch = (next() % 101) as u8;
                p.distortion = (next() % 101) as u8;
                p.age = (next() % 101) as u8;
                p.saturation = (next() % 101) as u8;
                p.wow = (next() % 101) as u8;
                p.flutter = (next() % 101) as u8;
                p.hiss = (next() % 101) as u8;
                p.preamp = (next() % 37) as i8 + PREAMP_MIN_DB;
                p.auto_normalize = next() % 2 == 0;
                p.track_gain = (next() % 400) as f32 / 10.0 - 24.0;
                p.loudness = next() % 2 == 0;
                p.eq.mode = match next() % 3 {
                    0 => EqMode::Off,
                    1 => EqMode::Three,
                    _ => EqMode::Seven,
                };
                for b in p.eq.seven.iter_mut() {
                    *b = (next() % 25) as i8 - 12;
                }
                for b in p.eq.three.iter_mut() {
                    *b = (next() % 25) as i8 - 12;
                }
                p.stereo = StereoMode::ALL[(next() % 3) as usize];
                p.width = (next() % (WIDTH_MAX as u64 + 1)) as u8;
            }
            for v in fx.by_ref().take(half_second) {
                assert!(v.is_finite(), "non-finite sample after {total} samples");
                peak = peak.max(v.abs());
                total += 1;
            }
        }
        let el = t.elapsed().as_secs_f64();
        eprintln!(
            "soak fx: {secs}s processed in {el:.1}s ({:.1}x realtime), peak {peak:.3}",
            secs as f64 / el
        );
        assert_eq!(total, sr as usize * 2 * secs);
        assert!(peak < 2.0, "peak {peak}");
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
    fn preamp_scales_the_signal_before_the_chain() {
        let clean = run(FxParams::default());
        let quiet = run(FxParams {
            preamp: -6,
            ..Default::default()
        });
        let ratio = rms(&quiet) / rms(&clean);
        assert!((ratio - 0.501).abs() < 0.01, "-6 dB gave ratio {ratio}");
        let loud = run(FxParams {
            preamp: 12,
            ..Default::default()
        });
        let peak = loud.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            peak <= 1.0 && peak > 0.9,
            "+12 dB peak {peak} should be limited"
        );
        // A boost into tape saturation changes the shape, not just the level.
        let tape = |pre| {
            run(FxParams {
                mode: Mode::ReelToReel,
                hiss: 0,
                wow: 0,
                flutter: 0,
                age: 0,
                preamp: pre,
                ..Default::default()
            })
        };
        let (a, b) = (tape(0), tape(12));
        let (ra, rb) = (rms(&a), rms(&b));
        assert!(rb > ra, "boost should be louder: {ra} vs {rb}");
        assert!(
            rb / ra < 3.98,
            "saturation should compress a +12 dB boost: {}",
            rb / ra
        );
        let muted = run(FxParams {
            preamp: PREAMP_MIN_DB,
            ..Default::default()
        });
        let ratio = rms(&muted) / rms(&clean);
        assert!((ratio - 0.063).abs() < 0.005, "-24 dB gave ratio {ratio}");
    }
    #[test]
    fn normalisation_adds_to_preamp_and_ramps() {
        let p = FxParams {
            preamp: 6,
            auto_normalize: true,
            track_gain: -12.0,
            ..Default::default()
        };
        assert_eq!(p.effective_preamp_db(), -6.0);
        assert_eq!(
            FxParams {
                auto_normalize: false,
                ..p
            }
            .effective_preamp_db(),
            6.0
        );
        assert_eq!(
            FxParams {
                track_gain: 40.0,
                ..p
            }
            .effective_preamp_db(),
            6.0 + NORMALIZE_RANGE_DB.1
        );
        // A gain that changes while streaming fades over a fraction of a
        // second instead of stepping.
        let shared = Arc::new(Mutex::new(FxParams {
            auto_normalize: true,
            ..Default::default()
        }));
        let data = vec![0.5f32; 44_100 * 2 * 3];
        let src = SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(44_100).unwrap(),
            data,
        );
        let mut fx = Fx::new(src, shared.clone(), None);
        let first: Vec<f32> = fx.by_ref().take(2000).collect();
        assert!(first.iter().all(|&v| v == 0.5));
        shared.lock().unwrap().track_gain = -6.0;
        let rest: Vec<f32> = fx.collect();
        let at = |ms: usize| rest[ms * 44_100 * 2 / 1000 + 512];
        assert!(
            at(5) < 0.5 && at(5) > 0.45,
            "ramp should have started: {}",
            at(5)
        );
        assert!(at(100) > at(300), "ramp should keep falling");
        let last = *rest.last().unwrap();
        assert!(
            (last - 0.2505).abs() < 0.002,
            "should settle at -6 dB: {last}"
        );
        let steps = rest
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(steps < 0.002, "largest step {steps} is a click");
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
