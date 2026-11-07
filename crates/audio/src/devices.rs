//! Output device lookup and opening.

use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::{self, BufferSize, Device, FromSample, SampleFormat, SizedSample, StreamConfig};
use rodio::mixer::{Mixer, MixerSource, mixer};
use rodio::stream::supported_output_configs;

use crate::dsp::Bus;
use crate::glitches::{CallbackClock, Glitches};
use crate::voice::Controls;

/// Names of the output devices, as accepted by `AudioEngine::new` and `set_device`.
pub fn output_devices() -> Vec<String> {
    let Ok(devices) = cpal::default_host().output_devices() else {
        return Vec::new();
    };
    devices.filter_map(|d| device_name(&d)).collect()
}

fn device_name(d: &Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_owned())
}

fn named_device(name: &str) -> Option<Device> {
    cpal::default_host()
        .output_devices()
        .ok()?
        .find(|d| device_name(d).as_deref() == Some(name))
}

/// An open output device: a rodio mixer fed by a cpal stream the engine starts and stops.
/// rodio's `MixerDeviceSink` keeps its stream private and always running, and a running
/// WASAPI stream wakes its thread every few milliseconds even when it only plays silence.
/// A new `Output` is stopped.
pub(crate) struct Output {
    mixer: Mixer,
    stream: Stream,
    playing: bool,
    glitches: Arc<Glitches>,
}

enum Stream {
    Device(cpal::Stream),
    /// Tests: a thread that consumes the mix in real time, with no device.
    #[cfg(test)]
    Null(null::Null),
}

impl Output {
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// Starts or stops the device callback. While stopped, the mixer and its sources don't
    /// advance, and the device keeps its buffer, so starting again is near-instant.
    pub fn set_playing(&mut self, on: bool) -> Result<(), String> {
        if on != self.playing {
            if on {
                self.glitches.restarted();
            }
            match &self.stream {
                Stream::Device(s) => if on { s.play() } else { s.pause() }
                    .map_err(|e| format!("audio device: {e}"))?,
                #[cfg(test)]
                Stream::Null(n) => n.set_playing(on),
            }
            self.playing = on;
        }
        Ok(())
    }
}

/// Why an output didn't open.
#[derive(Debug)]
pub(crate) enum OpenError {
    /// There is no output device at all: nothing is plugged in, or it's switched off.
    NoDevice,
    /// A device is there but didn't open.
    Failed(String),
}

impl From<String> for OpenError {
    fn from(e: String) -> Self {
        OpenError::Failed(e)
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NoDevice => f.write_str("no audio output device"),
            OpenError::Failed(e) => f.write_str(e),
        }
    }
}

/// Opens `name`, or the default device when `name` is `None`, unknown, or fails to open.
/// `on_error` runs on the audio thread for every cpal stream error. The mix goes through
/// the master bus `controls` set (`dsp`); late callbacks are counted in its glitches.
pub(crate) fn open_output<E>(
    name: Option<&str>,
    on_error: E,
    controls: &Arc<Controls>,
) -> Result<Output, OpenError>
where
    E: FnMut(cpal::Error) + Send + Clone + 'static,
{
    if let Some(device) = name.and_then(named_device)
        && let Ok(out) = open_device(&device, on_error.clone(), controls)
    {
        return Ok(out);
    }
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(OpenError::NoDevice)?;
    Ok(open_device(&device, on_error, controls)?)
}

/// The device's default format with a ~50 ms buffer, else its other formats; as rodio's
/// `DeviceSinkBuilder::from_device(..).open_sink_or_fallback()` does.
fn open_device<E>(device: &Device, on_error: E, controls: &Arc<Controls>) -> Result<Output, String>
where
    E: FnMut(cpal::Error) + Send + Clone + 'static,
{
    let default = device
        .default_output_config()
        .map_err(|e| format!("audio device: {e}"))?;
    let mut config = default.config();
    config.buffer_size = BufferSize::Fixed(power_of_two_near(config.sample_rate / 20));
    let first = match build(
        device,
        config,
        default.sample_format(),
        on_error.clone(),
        controls,
    ) {
        Ok(out) => return Ok(out),
        Err(e) => e,
    };
    let others = supported_output_configs(device).map_err(|_| first.clone())?;
    for c in others {
        if let Ok(out) = build(
            device,
            c.config(),
            c.sample_format(),
            on_error.clone(),
            controls,
        ) {
            return Ok(out);
        }
    }
    Err(first)
}

fn build<E>(
    device: &Device,
    config: StreamConfig,
    format: SampleFormat,
    on_error: E,
    controls: &Arc<Controls>,
) -> Result<Output, String>
where
    E: FnMut(cpal::Error) + Send + 'static,
{
    let (Some(channels), Some(rate)) = (
        NonZero::new(config.channels),
        NonZero::new(config.sample_rate),
    ) else {
        return Err("audio device: empty format".into());
    };
    let (mixer, source) = mixer(channels, rate);
    let clock = CallbackClock::new(buffer_duration(&config));
    let bus = Bus::new(controls.clone(), channels.get(), rate.get());
    let g = controls.glitches.clone();
    let stream = match format {
        SampleFormat::F32 => stream::<f32, E>(device, config, source, bus, on_error, clock, g),
        SampleFormat::I16 => stream::<i16, E>(device, config, source, bus, on_error, clock, g),
        SampleFormat::I32 => stream::<i32, E>(device, config, source, bus, on_error, clock, g),
        SampleFormat::U16 => stream::<u16, E>(device, config, source, bus, on_error, clock, g),
        f => return Err(format!("audio device: unsupported sample format {f}")),
    }
    .map_err(|e| format!("audio device: {e}"))?;
    Ok(Output {
        mixer,
        stream: Stream::Device(stream),
        playing: false,
        glitches: controls.glitches.clone(),
    })
}

/// How long the device plays from one full buffer. A format without a fixed size (a
/// fallback) gets WASAPI's shared-mode default, about the same ~50 ms.
fn buffer_duration(config: &StreamConfig) -> Duration {
    let frames = match config.buffer_size {
        BufferSize::Fixed(n) => n,
        BufferSize::Default => power_of_two_near(config.sample_rate / 20),
    };
    Duration::from_secs_f64(f64::from(frames) / f64::from(config.sample_rate.max(1)))
}

fn stream<T, E>(
    device: &Device,
    config: StreamConfig,
    mut source: MixerSource,
    mut bus: Bus,
    on_error: E,
    mut clock: CallbackClock,
    glitches: Arc<Glitches>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<rodio::Sample>,
    E: FnMut(cpal::Error) + Send + 'static,
{
    let mut task: Option<Option<crate::priority::ProAudio>> = None;
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            // On the first callback: this is cpal's thread for the stream's whole life.
            if task.is_none() {
                task = Some(crate::priority::pro_audio());
            }
            clock.tick(&glitches);
            bus.fill(&mut source, data);
        },
        on_error,
        None,
    )
}

/// The power of two nearest `n`, as rodio rounds its default buffer size.
fn power_of_two_near(n: u32) -> u32 {
    let n = n.max(1);
    let next = n.next_power_of_two();
    let prev = (next >> 1).max(1);
    if n - prev <= next - n { prev } else { next }
}

/// Errors after which the stream is dead and must be reopened. Others (xruns, a denied
/// real-time promotion, an automatic reroute) leave it playing.
pub(crate) fn is_device_lost(e: &cpal::Error) -> bool {
    matches!(
        e.kind(),
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated
    )
}

/// A null output for tests: 44.1 kHz stereo through the master bus, consumed in real time
/// in ~10 ms steps while playing.
#[cfg(test)]
pub(crate) fn open_null(controls: &Arc<Controls>) -> Output {
    let (mixer, source) = mixer(NonZero::new(2).unwrap(), NonZero::new(44_100).unwrap());
    // The buffer `open_device` asks for at 44.1 kHz (2048 frames, 46 ms).
    let buffer = Duration::from_secs_f64(f64::from(power_of_two_near(44_100 / 20)) / 44_100.0);
    let clock = CallbackClock::new(buffer);
    let bus = Bus::new(controls.clone(), 2, 44_100);
    Output {
        mixer,
        stream: Stream::Null(null::Null::start(
            source,
            bus,
            44_100,
            clock,
            controls.glitches.clone(),
        )),
        playing: false,
        glitches: controls.glitches.clone(),
    }
}

#[cfg(test)]
mod null {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use rodio::mixer::MixerSource;

    use crate::dsp::Bus;
    use crate::glitches::{CallbackClock, Glitches};

    /// Stereo frames consumed per step at most; a longer gap takes several.
    const STEP_FRAMES: usize = 2048;

    pub struct Null {
        playing: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Null {
        /// Consumes stereo `source` through `bus` at `frames_per_s`. Paced by the clock,
        /// not by counting sleeps: on a loaded machine a 10 ms sleep can last ~100 ms, and
        /// fixed steps then played at a tenth of real time.
        /// Each step counts as a callback on `clock`, as a device's would.
        pub fn start(
            mut source: MixerSource,
            mut bus: Bus,
            frames_per_s: u32,
            mut clock: CallbackClock,
            glitches: Arc<Glitches>,
        ) -> Null {
            let playing = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (p, s) = (playing.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                // As a device's thread: see `stream`.
                let _task = crate::priority::pro_audio();
                let mut out = vec![0f32; 2 * STEP_FRAMES];
                let mut last = Instant::now();
                // Frames due but not consumed yet (a fraction of one, between steps).
                let mut owed = 0.0;
                while !s.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if p.load(Ordering::Relaxed) {
                        clock.tick(&glitches);
                        owed += (now - last).as_secs_f64() * f64::from(frames_per_s);
                        while owed >= 1.0 {
                            let n = (owed as usize).min(STEP_FRAMES);
                            bus.fill(&mut source, &mut out[..2 * n]);
                            owed -= n as f64;
                        }
                    } else {
                        owed = 0.0;
                    }
                    last = now;
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            Null {
                playing,
                stop,
                thread: Some(thread),
            }
        }

        pub fn set_playing(&self, on: bool) {
            self.playing.store(on, Ordering::Relaxed);
        }
    }

    impl Drop for Null {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            // As cpal's stream does: once dropped, nothing reads the mix any more, so a
            // voice's source never has two readers.
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}
