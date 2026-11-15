mod biquad;
mod channel;
mod derived;
mod eq_mode;
mod eq_params;
mod fx_mode;
mod fx_params;
mod rng;
mod stereo_mode;
#[cfg(test)]
mod tests;
use crate::spectrum::SharedTap;
use biquad::Biquad;
use channel::Channel;
use derived::Derived;
pub use eq_mode::{EqMode, SEVEN_BANDS, THREE_BANDS};
pub use eq_params::{EQ_RANGE_DB, EqParams};
pub use fx_mode::FxMode;
pub use fx_params::{
    FxParams, NORMALIZE_RANGE_DB, NORMALIZE_TARGET_DB, PREAMP_MAX_DB, PREAMP_MIN_DB, SharedParams,
};
use rng::Rng;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::time::Duration;
pub use stereo_mode::{StereoMode, WIDTH_MAX};
/// Time constant of the ramp between preamp gains, so a measurement that
/// arrives mid-track fades in rather than steps.
const GAIN_RAMP_SECS: f32 = 0.3;
const REFRESH_FRAMES: u64 = 256;
const BASE_DELAY_SECS: f32 = 0.016;
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
            FxMode::Off => {}
            FxMode::Vinyl => {
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
            FxMode::ReelToReel | FxMode::Cassette => {
                let cassette = p.mode == FxMode::Cassette;
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
        if d.mode == FxMode::Vinyl {
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
        if d.mode == FxMode::Off && !shape && gain == 1.0 {
            return x;
        }
        let mut y = x * gain;
        let sr = self.sr;
        let c = &mut self.chans[ch];
        if d.mode == FxMode::Off {
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
        if d.mode != FxMode::Off {
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
