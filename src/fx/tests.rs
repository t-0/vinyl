use super::*;
use rodio::buffer::SamplesBuffer;
use std::num::NonZero;
use std::sync::{Arc, Mutex};
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
fn clean_tape(mode: FxMode, hiss: u8) -> FxParams {
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
    for mode in [FxMode::ReelToReel, FxMode::Cassette] {
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
        clean_tape(FxMode::ReelToReel, 50),
        silence.clone(),
        44_100,
    ));
    let cass = rms(&run_signal(
        clean_tape(FxMode::Cassette, 50),
        silence,
        44_100,
    ));
    assert!(cass > reel * 2.0, "cassette {cass} vs reel {reel}");
}
#[test]
fn age_still_raises_hiss() {
    let silence = vec![0.0f32; 44_100 * 2];
    let young = rms(&run_signal(
        clean_tape(FxMode::ReelToReel, 50),
        silence.clone(),
        44_100,
    ));
    let old = rms(&run_signal(
        FxParams {
            age: 100,
            ..clean_tape(FxMode::ReelToReel, 50)
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
    assert_eq!(p.mode, FxMode::Cassette);
    let round: FxParams = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(round, p);
}
#[test]
fn vinyl_scratch_zero_is_free_of_noise_and_pops() {
    let silence = vec![0.0f32; 44_100 * 2];
    let out = run_signal(
        FxParams {
            mode: FxMode::Vinyl,
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
        mode: FxMode::Cassette,
        ..Default::default()
    };
    let out: Vec<f32> = Fx::new(src, Arc::new(Mutex::new(p)), None).collect();
    assert_eq!(out.len(), 22_050);
    assert!(out.iter().all(|v| v.is_finite()));
}
#[test]
fn parameter_changes_apply_while_streaming() {
    let shared = Arc::new(Mutex::new(FxParams {
        mode: FxMode::Off,
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
    shared.lock().unwrap().mode = FxMode::Cassette;
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
    for mode in [FxMode::Vinyl, FxMode::ReelToReel, FxMode::Cassette] {
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
            p.mode = FxMode::ALL[(next() % FxMode::ALL.len() as u64) as usize];
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
        mode: FxMode::Off,
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
            mode: FxMode::ReelToReel,
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
    for mode in [FxMode::Vinyl, FxMode::ReelToReel, FxMode::Cassette] {
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
        mode: FxMode::Cassette,
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
