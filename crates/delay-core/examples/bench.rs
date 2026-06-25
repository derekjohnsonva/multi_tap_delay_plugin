//! Quick throughput benchmark for the engine hot path (not a unit test).
//!
//! No args runs the full table (16/64/128 taps, 48 kHz) and reports the
//! real-time factor (×RT) for each:
//!     cargo run --release --example bench -p delay-core
//!
//! Pass `<taps> [seconds]` to run a single steady-state config for longer —
//! the focused, long-running form to point a profiler / flamegraph at the
//! max-taps hot path (see the README "Profiling" section):
//!     cargo run --release --example bench -p delay-core -- 128 60

use delay_core::{Engine, LaneSource, Tap};
use std::time::Instant;

const SR: f32 = 48_000.0;
const MAX_DELAY: usize = (SR as usize) * 30; // 30 s, matches the plugin
const BLOCK: usize = 512;

fn run(label: &str, num_taps: usize, settled: bool, seconds: f32) {
    let mut eng = Engine::new(SR, MAX_DELAY);
    eng.reserve_taps(num_taps);
    eng.set_smoothing_ms(20.0);
    eng.set_mix(0.5);
    eng.set_output_trim(1.0);

    // A pan lane that actually exercises equal-power across taps (ping-pong),
    // and an exponential-decay amplitude — i.e. realistic per-tap coefficients.
    let pan = LaneSource::PingPong {
        width: 0.9,
        widen: 0.5,
    };
    let amp = LaneSource::ExpDecay { k: 2.0 };
    let step = (0.011 * SR).round(); // ~11 ms tap spacing -> taps fill the buffer
    let taps: Vec<Tap> = (0..num_taps)
        .map(|i| {
            let x = if num_taps > 1 {
                i as f32 / (num_taps - 1) as f32
            } else {
                0.0
            };
            Tap::new((i as f32 + 1.0) * step, amp.value_at(x), pan.value_at(x))
        })
        .collect();
    eng.set_taps(&taps);

    if settled {
        // Let every smoother arrive at its target so we measure steady state.
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        for _ in 0..200 {
            eng.process(&mut l, &mut r);
        }
    }

    // Time a fixed amount of audio.
    let total = (SR * seconds) as usize;
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    // Deterministic pseudo-noise input.
    let mut phase = 0.0f32;
    let start = Instant::now();
    let mut done = 0;
    while done < total {
        for i in 0..BLOCK {
            phase += 0.01;
            let s = (phase).sin() * 0.5;
            l[i] = s;
            r[i] = -s;
        }
        eng.process(&mut l, &mut r);
        done += BLOCK;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let rt = seconds as f64 / elapsed;
    // Sink to prevent the optimizer from eliding the loop.
    let sink: f32 = l.iter().chain(r.iter()).sum();
    println!(
        "{label:<28} taps={num_taps:<4} {elapsed:7.3}s for {seconds:.0}s audio  ->  {rt:7.1}x RT   (sink={sink:.3})"
    );
}

fn main() {
    // `bench <taps> [seconds]` -> one focused, long steady-state run for
    // profiling; otherwise the standard comparison table.
    let mut args = std::env::args().skip(1);
    if let Some(taps) = args.next().and_then(|s| s.parse::<usize>().ok()) {
        let seconds = args
            .next()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(60.0);
        println!("--- focused profiling run ---");
        run("steady-state", taps, true, seconds);
        return;
    }

    println!("--- engine throughput (higher xRT = faster) ---");
    for &n in &[16usize, 64, 128] {
        run("steady-state", n, true, 10.0);
    }
    run("ramping (unsettled)", 128, false, 10.0);
}
