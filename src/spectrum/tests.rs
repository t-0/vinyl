use super::tap::RING;
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
    let (best, _) = mags.iter().enumerate().fold(
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
    let expected = ((1000.0f32 / 40.0).ln() / (16_000.0f32 / 40.0).ln() * BANDS as f32) as usize;
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
