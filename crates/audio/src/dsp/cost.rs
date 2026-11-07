//! What the bus costs with the equalizer (Bass) and the compressor (Strong) on: 60 s of
//! 48 kHz stereo noise through it in 10 ms callbacks, timed on this thread's CPU clock. The
//! 60 s go through `PASSES` times: the clock ticks in 15.6 ms steps, and one pass takes only
//! about two of them. A measurement, run by hand in release on an otherwise quiet machine:
//!
//! ```text
//! RUSTFLAGS="--cfg ci" cargo nextest run -p featherplay-audio --lib --release \
//!     -E 'test(dsp::cost::)' --run-ignored only --no-capture
//! ```

use std::time::{Duration, Instant};

use super::{Chain, Compressor};

const RATE: u32 = 48_000;
const SECONDS: usize = 60;
const PASSES: usize = 10;
/// A 10 ms callback.
const CALLBACK_FRAMES: usize = 480;

/// This thread's CPU time so far.
#[cfg(windows)]
fn cpu_time() -> Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle of the current thread is always valid, and the four
    // pointers are to live locals.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    assert!(ok.is_ok(), "GetThreadTimes");
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    // 100 ns ticks.
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

#[cfg(not(windows))]
fn cpu_time() -> Duration {
    Duration::ZERO
}

#[test]
#[ignore = "a measurement: run it in release, see the module docs"]
fn bass_and_strong_cost_per_second_of_audio() {
    // White noise at about −10 dBFS from a xorshift generator: no dependency, and the same
    // every run.
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let mut noise = vec![0f32; RATE as usize * 2];
    for s in &mut noise {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *s = ((x >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.6;
    }
    // Bass, as asked; then every band moved, the most the equalizer does.
    for (name, bands) in [
        ("Bass + Strong", [8.0, 0.0, 0.0, 0.0, 0.0]),
        ("five bands + Strong", [6.0, -3.0, 2.0, 4.0, 5.0]),
    ] {
        let mut chain = Chain::new(2, RATE);
        chain.set(Some(bands), Compressor::Strong);
        let (cpu0, wall0) = (cpu_time(), Instant::now());
        for _ in 0..SECONDS * PASSES {
            for chunk in noise.as_chunks::<{ CALLBACK_FRAMES * 2 }>().0 {
                let mut block = *chunk;
                chain.process(&mut block, 0.7);
                std::hint::black_box(&block);
            }
        }
        let (cpu, wall) = (cpu_time() - cpu0, wall0.elapsed());
        let per_s = |d: Duration| d.as_secs_f64() * 1000.0 / (SECONDS * PASSES) as f64;
        println!(
            "bus, {name}, 48 kHz stereo: {:.2} ms of CPU per second of audio ({:.2} ms wall)",
            per_s(cpu),
            per_s(wall)
        );
    }
}
