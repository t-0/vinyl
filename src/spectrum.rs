use std::sync::{Arc, Mutex};
pub struct Tap {
    pub sample_rate: f32,
    ring: Vec<f32>,
    pos: usize,
}
pub type SharedTap = Arc<Mutex<Tap>>;
pub const FFT_SIZE: usize = 2048;
pub const BANDS: usize = 64;
const RING: usize = FFT_SIZE * 2;
impl Tap {
    pub fn new() -> SharedTap {
        Arc::new(Mutex::new(Tap {
            sample_rate: 44_100.0,
            ring: vec![0.0; RING],
            pos: 0,
        }))
    }
    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.pos] = s;
            self.pos = (self.pos + 1) % RING;
        }
    }
    pub fn latest(&self, n: usize, out: &mut Vec<f32>) {
        out.clear();
        let n = n.min(RING);
        let start = (self.pos + RING - n) % RING;
        for i in 0..n {
            out.push(self.ring[(start + i) % RING]);
        }
    }
}
pub struct Analyser {
    pub levels: [f32; BANDS],
    pub peaks: [f32; BANDS],
    samples: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
    edges: [(usize, usize); BANDS],
    edges_rate: f32,
}
impl Default for Analyser {
    fn default() -> Self {
        Self::new()
    }
}
impl Analyser {
    pub fn new() -> Self {
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let mut a = Self {
            levels: [0.0; BANDS],
            peaks: [0.0; BANDS],
            samples: Vec::with_capacity(FFT_SIZE),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            window,
            edges: [(0, 0); BANDS],
            edges_rate: 0.0,
        };
        a.compute_edges(44_100.0);
        a
    }
    fn compute_edges(&mut self, sr: f32) {
        let bin_hz = sr / FFT_SIZE as f32;
        let (lo, hi) = (40.0f32, (16_000.0f32).min(sr * 0.45));
        let ratio = hi / lo;
        let mut prev_end = (lo / bin_hz).max(1.0) as usize;
        for b in 0..BANDS {
            let f_end = lo * ratio.powf((b + 1) as f32 / BANDS as f32);
            let mut end = (f_end / bin_hz).round() as usize;
            if end <= prev_end {
                end = prev_end + 1;
            }
            let end = end.min(FFT_SIZE / 2);
            self.edges[b] = (prev_end, end.max(prev_end + 1));
            prev_end = end;
        }
        self.edges_rate = sr;
    }
    pub fn update(&mut self, tap: Option<&SharedTap>, active: bool) {
        let mut fresh = [0.0f32; BANDS];
        if active
            && let Some(tap) = tap
            && let Ok(t) = tap.lock()
        {
            let sr = t.sample_rate;
            t.latest(FFT_SIZE, &mut self.samples);
            drop(t);
            if (sr - self.edges_rate).abs() > 1.0 {
                self.compute_edges(sr);
            }
            if self.samples.len() == FFT_SIZE {
                for i in 0..FFT_SIZE {
                    self.re[i] = self.samples[i] * self.window[i];
                    self.im[i] = 0.0;
                }
                fft(&mut self.re, &mut self.im);
                let norm = 4.0 / FFT_SIZE as f32;
                for (b, &(s, e)) in self.edges.iter().enumerate() {
                    let mut peak = 0.0f32;
                    for k in s..e {
                        let m = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() * norm;
                        peak = peak.max(m);
                    }
                    let db = 20.0 * (peak + 1e-9).log10() + b as f32 * 0.2;
                    fresh[b] = ((db + 54.0) / 54.0).clamp(0.0, 1.0);
                }
            }
        }
        for ((level, peak), &target) in self
            .levels
            .iter_mut()
            .zip(self.peaks.iter_mut())
            .zip(fresh.iter())
        {
            let cur = *level;
            *level = if target > cur {
                cur + (target - cur) * 0.7
            } else {
                cur + (target - cur) * 0.3
            };
            *peak = (*peak - 0.025).max(*level).max(0.0);
        }
    }
}
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..half {
                let (ar, ai) = (re[i + k], im[i + k]);
                let (br, bi) = (re[i + k + half], im[i + k + half]);
                let tr = br * cr - bi * ci;
                let ti = br * ci + bi * cr;
                re[i + k] = ar + tr;
                im[i + k] = ai + ti;
                re[i + k + half] = ar - tr;
                im[i + k + half] = ai - ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_finds_a_sine() {
        let n = FFT_SIZE;
        let k = 100;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * k as f32 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..n / 2)
            .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
            .collect();
        let (best, _) =
            mags.iter().enumerate().fold(
                (0, 0.0),
                |(bi, bm), (i, &m)| if m > bm { (i, m) } else { (bi, bm) },
            );
        assert_eq!(best, k);
        assert!((mags[k] - n as f32 / 2.0).abs() < 1.0);
    }
    #[test]
    fn analyser_lights_the_right_band() {
        let tap = Tap::new();
        let sr = 44_100.0;
        let tone: Vec<f32> = (0..RING)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin() * 0.5)
            .collect();
        tap.lock().unwrap().push(&tone);
        let mut a = Analyser::new();
        for _ in 0..5 {
            a.update(Some(&tap), true);
        }
        let (best, _) =
            a.levels.iter().enumerate().fold(
                (0, 0.0),
                |(bi, bm), (i, &m)| if m > bm { (i, m) } else { (bi, bm) },
            );
        let expected =
            ((1000.0f32 / 40.0).ln() / (16_000.0f32 / 40.0).ln() * BANDS as f32) as usize;
        assert!(
            (best as i64 - expected as i64).abs() <= 1,
            "best {best} expected {expected}"
        );
        assert!(a.levels[best] > 0.6);
        for _ in 0..40 {
            a.update(Some(&tap), false);
        }
        assert!(
            a.levels.iter().all(|&l| l < 0.01),
            "did not decay when inactive"
        );
    }
}
