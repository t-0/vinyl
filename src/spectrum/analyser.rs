use super::{FFT_SIZE, SharedTap, fft};
pub const BANDS: usize = 64;
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
