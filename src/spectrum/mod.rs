mod analyser;
mod tap;
#[cfg(test)]
mod tests;
pub use analyser::{Analyser, BANDS};
pub use tap::{SharedTap, Tap};
pub const FFT_SIZE: usize = 2048;
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
