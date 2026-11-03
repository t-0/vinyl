use crate::fx::{Fx, FxParams, SharedParams};
use crate::spectrum::{SharedTap, Tap};
use anyhow::{Context, Result};
use rodio::cpal::BufferSize;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
fn open_output() -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
    let first = DeviceSinkBuilder::from_default_device()
        .and_then(|b| b.with_buffer_size(BufferSize::Default).open_stream());
    match first {
        Ok(s) => Ok(s),
        Err(_) => DeviceSinkBuilder::open_default_sink(),
    }
}
pub struct Audio {
    _stream: MixerDeviceSink,
    player: Player,
    fx: SharedParams,
    tap: SharedTap,
}
impl Audio {
    pub fn new(fx: FxParams) -> Result<Self> {
        let mut stream = open_output().context("opening default audio output")?;
        stream.log_on_drop(false);
        let player = Player::connect_new(stream.mixer());
        if std::env::var_os("VINYL_MUTE").is_some() {
            player.set_volume(0.0);
        }
        Ok(Self {
            _stream: stream,
            player,
            fx: Arc::new(Mutex::new(fx)),
            tap: Tap::new(),
        })
    }
    pub fn load(&mut self, path: &Path, speed: f32) -> Result<()> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let len = file.metadata().map(|m| m.len()).ok();
        let mut builder = Decoder::builder()
            .with_data(BufReader::new(file))
            .with_seekable(true);
        if let Some(len) = len {
            builder = builder.with_byte_len(len);
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            builder = builder.with_hint(ext);
        }
        let source = builder
            .build()
            .with_context(|| format!("decoding {}", path.display()))?;
        let source = Fx::new(source, self.fx.clone(), Some(self.tap.clone()));
        self.player.clear();
        self.player.set_speed(speed);
        self.player.append(source);
        self.player.play();
        Ok(())
    }
    pub fn tap(&self) -> &SharedTap {
        &self.tap
    }
    pub fn set_fx(&self, p: FxParams) {
        if let Ok(mut g) = self.fx.lock() {
            *g = p;
        }
    }
    pub fn toggle_pause(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }
    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }
    pub fn finished(&self) -> bool {
        self.player.empty()
    }
    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }
    pub fn set_speed(&self, speed: f32) {
        self.player.set_speed(speed);
    }
    pub fn seek_by(&self, delta_secs: f64, total_secs: f64) {
        let cur = self.position().as_secs_f64();
        let mut target = cur + delta_secs;
        if target < 0.0 {
            target = 0.0;
        }
        if total_secs > 0.0 && target > total_secs - 0.5 {
            target = (total_secs - 0.5).max(0.0);
        }
        let _ = self.player.try_seek(Duration::from_secs_f64(target));
    }
}
