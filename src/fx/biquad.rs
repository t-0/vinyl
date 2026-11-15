#[derive(Clone, Copy, Default)]
pub(super) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}
impl Biquad {
    pub(super) fn set_coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    pub(super) fn set_peaking(&mut self, sr: f32, fc: f32, q: f32, gain_db: f32) {
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
    pub(super) fn set_lowshelf(&mut self, sr: f32, fc: f32, gain_db: f32) {
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
    pub(super) fn set_highshelf(&mut self, sr: f32, fc: f32, gain_db: f32) {
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
    pub(super) fn set_lowpass(&mut self, sr: f32, fc: f32) {
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
    pub(super) fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
