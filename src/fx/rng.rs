pub(super) struct Rng(pub(super) u32);
impl Rng {
    pub(super) fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    pub(super) fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
    pub(super) fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}
