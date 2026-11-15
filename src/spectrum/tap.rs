use super::FFT_SIZE;
use std::sync::{Arc, Mutex};
pub struct Tap {
    pub sample_rate: f32,
    ring: Vec<f32>,
    pos: usize,
}
pub type SharedTap = Arc<Mutex<Tap>>;
pub(super) const RING: usize = FFT_SIZE * 2;
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
