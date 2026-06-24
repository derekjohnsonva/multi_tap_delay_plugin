//! PR 3/4/6 — The multi-tap engine.
//!
//! One stereo delay buffer, N read taps. Each tap reads at a fractional delay,
//! is collapsed to mono, scaled by a (smoothed) gain, and equal-power panned
//! into the stereo field. The summed wet signal is crossfaded against the dry
//! input by `mix` and scaled by `output_trim` (design doc §4).
//!
//! No feedback ⇒ unconditionally stable ⇒ arbitrary per-tap gains are safe.

use crate::buffer::DelayLine;
use crate::limiter::Limiter;
use crate::pan::equal_power;
use crate::smoothing::OnePole;

/// A target configuration for one tap. This is the plain-data description the
/// host/GUI hands to the engine; the engine keeps its own smoothed copy.
#[derive(Clone, Copy, Debug)]
pub struct Tap {
    /// Read position in samples (fractional allowed).
    pub delay_samples: f32,
    /// Linear amplitude. `0..1` normally; negative flips polarity (advanced).
    pub gain: f32,
    /// Pan position, `-1.0` (hard L) .. `+1.0` (hard R).
    pub pan: f32,
}

impl Tap {
    pub fn new(delay_samples: f32, gain: f32, pan: f32) -> Self {
        Self {
            delay_samples,
            gain,
            pan,
        }
    }
}

/// Internal per-tap state: the target plus smoothers for the coefficients that
/// would otherwise zipper (gain, pan). Delay time is not smoothed here — time
/// modulation is a future lane (design doc §8); changing it abruptly is fine
/// for now because taps only modulate amplitude.
struct TapState {
    delay_samples: f32,
    gain: OnePole,
    pan: OnePole,
    /// A removed tap that is fading its gain to zero before being dropped, so
    /// the tap-count decrease doesn't click (design §3).
    dying: bool,
}

/// The multi-tap delay engine. Allocate once with [`Engine::new`]; all
/// per-sample work is allocation-free.
pub struct Engine {
    sample_rate: f32,
    left: DelayLine,
    right: DelayLine,
    taps: Vec<TapState>,
    mix: OnePole,
    output_trim: OnePole,
    limiter: Limiter,
    smoothing_ms: f32,
    /// Decaying peak of the post-trim output (max across L/R), for the editor's
    /// always-visible output meter (design §4/§7). Updated per sample; rises
    /// instantly to a new peak and falls back by `meter_release` each sample.
    meter_peak: f32,
    meter_release: f32,
}

/// Time for the output meter's peak hold to decay by `1/e` (≈37%). A slowish
/// release keeps transient peaks readable without latching.
const METER_RELEASE_SECONDS: f32 = 0.3;

/// Per-sample multiplier giving the [`METER_RELEASE_SECONDS`] decay at `sr`.
fn meter_release_coeff(sample_rate: f32) -> f32 {
    (-1.0 / (METER_RELEASE_SECONDS * sample_rate)).exp()
}

impl Engine {
    /// Create an engine. `max_delay_samples` bounds the longest tap time.
    pub fn new(sample_rate: f32, max_delay_samples: usize) -> Self {
        let mut mix = OnePole::new(1.0);
        let mut output_trim = OnePole::new(1.0);
        let smoothing_ms = 20.0;
        mix.set_time(smoothing_ms, sample_rate);
        output_trim.set_time(smoothing_ms, sample_rate);
        Self {
            sample_rate,
            left: DelayLine::new(max_delay_samples),
            right: DelayLine::new(max_delay_samples),
            taps: Vec::new(),
            mix,
            output_trim,
            limiter: Limiter::new(sample_rate),
            smoothing_ms,
            meter_peak: 0.0,
            meter_release: meter_release_coeff(sample_rate),
        }
    }

    /// Clear all audio state (buffers + smoother positions snap to target).
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.limiter.reset();
        self.meter_peak = 0.0;
        for t in &mut self.taps {
            t.gain.set_immediate(t.gain.value());
            t.pan.set_immediate(t.pan.value());
        }
    }

    /// Current decaying peak of the post-trim output (linear, max of L/R). The
    /// editor reads this for the output meter; it's `0.0` until audio flows.
    pub fn output_level(&self) -> f32 {
        self.meter_peak
    }

    /// Set the per-coefficient smoothing time (ms) for gain, pan and mix.
    pub fn set_smoothing_ms(&mut self, time_ms: f32) {
        self.smoothing_ms = time_ms;
        for t in &mut self.taps {
            t.gain.set_time(time_ms, self.sample_rate);
            t.pan.set_time(time_ms, self.sample_rate);
        }
        self.mix.set_time(time_ms, self.sample_rate);
        self.output_trim.set_time(time_ms, self.sample_rate);
    }

    /// Dry/wet balance, `0.0` (dry only) .. `1.0` (wet only).
    pub fn set_mix(&mut self, mix: f32) {
        self.mix.set_target(mix.clamp(0.0, 1.0));
    }

    /// Linear output gain applied after the dry/wet mix.
    pub fn set_output_trim(&mut self, gain: f32) {
        self.output_trim.set_target(gain.max(0.0));
    }

    /// Enable/disable the optional safety limiter on the summed wet signal.
    pub fn set_limiter_enabled(&mut self, enabled: bool) {
        self.limiter.set_enabled(enabled);
    }

    /// Number of tap slots the engine is processing, including any that are
    /// fading out after removal.
    pub fn num_taps(&self) -> usize {
        self.taps.len()
    }

    /// Pre-allocate capacity for `max_taps` so growing the tap set in
    /// [`Engine::set_taps`] never allocates on the audio thread. Call once when
    /// the host max tap count is known (e.g. from `initialize`).
    pub fn reserve_taps(&mut self, max_taps: usize) {
        if max_taps > self.taps.len() {
            self.taps.reserve(max_taps - self.taps.len());
        }
    }

    /// Replace the tap set. Existing taps (by index) keep their smoother state
    /// so gain/pan ramp from where they were; appended taps fade in from gain 0;
    /// removed taps (indices past the new length) fade their gain to 0 before
    /// being dropped, so neither a count increase nor decrease clicks (§3).
    pub fn set_taps(&mut self, taps: &[Tap]) {
        // Garbage-collect trailing taps that have finished fading out.
        while let Some(last) = self.taps.last() {
            if last.dying && last.gain.value().abs() < 1e-4 {
                self.taps.pop();
            } else {
                break;
            }
        }

        // Update existing slots or append new fade-in taps.
        for (i, tap) in taps.iter().enumerate() {
            if let Some(state) = self.taps.get_mut(i) {
                state.dying = false;
                state.delay_samples = tap.delay_samples;
                state.gain.set_target(tap.gain);
                state.pan.set_target(tap.pan);
            } else {
                let mut gain = OnePole::new(0.0); // fade in from silence
                let mut pan = OnePole::new(tap.pan);
                gain.set_time(self.smoothing_ms, self.sample_rate);
                pan.set_time(self.smoothing_ms, self.sample_rate);
                gain.set_target(tap.gain);
                self.taps.push(TapState {
                    delay_samples: tap.delay_samples,
                    gain,
                    pan,
                    dying: false,
                });
            }
        }

        // Any remaining slots are removed taps: fade them out, drop later.
        for state in self.taps.iter_mut().skip(taps.len()) {
            state.dying = true;
            state.gain.set_target(0.0);
        }
    }

    /// Process one stereo frame, returning the mixed `(left, right)` output.
    #[inline]
    pub fn process_sample(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        self.left.write(in_l);
        self.right.write(in_r);

        let mut wet_l = 0.0;
        let mut wet_r = 0.0;
        for tap in &mut self.taps {
            let gain = tap.gain.next();
            let pan = tap.pan.next();
            // Collapse the tap's stereo read to mono, then position it. This
            // makes pan a true position (needed for crisp ping-pong) rather
            // than a balance that merely attenuates one side.
            let src = 0.5 * (self.left.read(tap.delay_samples) + self.right.read(tap.delay_samples));
            let (lg, rg) = equal_power(pan);
            wet_l += src * gain * lg;
            wet_r += src * gain * rg;
        }

        // Optional safety limiter on the summed wet signal (before the dry mix,
        // so the dry path stays untouched). A no-op when disabled.
        let (wet_l, wet_r) = self.limiter.process(wet_l, wet_r);

        let mix = self.mix.next();
        let trim = self.output_trim.next();
        let out_l = (in_l * (1.0 - mix) + wet_l * mix) * trim;
        let out_r = (in_r * (1.0 - mix) + wet_r * mix) * trim;

        // Peak-hold meter: jump up to a new peak, otherwise decay.
        let peak = out_l.abs().max(out_r.abs());
        self.meter_peak = peak.max(self.meter_peak * self.meter_release);

        (out_l, out_r)
    }

    /// Convenience: process two equal-length channel slices in place.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (out_l, out_r) = self.process_sample(*l, *r);
            *l = out_l;
            *r = out_r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Build an engine with already-settled coefficients so single-sample
    /// assertions aren't fighting the smoother ramp.
    fn settled_engine(taps: &[Tap], mix: f32) -> Engine {
        let mut eng = Engine::new(SR, 4_096);
        eng.set_smoothing_ms(0.0); // instant
        eng.set_mix(mix);
        eng.set_output_trim(1.0);
        eng.set_taps(taps);
        eng
    }

    #[test]
    fn single_centered_tap_reproduces_delayed_impulse() {
        // One tap at 10 samples, gain 0.5, centered.
        let mut eng = settled_engine(&[Tap::new(10.0, 0.5, 0.0)], 1.0);
        // Impulse in.
        let (l, r) = eng.process_sample(1.0, 1.0);
        // Wet-only, mono source = 1.0, gain 0.5, center ≈ 0.707 each side.
        // At t=0 the tap (delay 10) hasn't fired yet -> silence.
        assert!(l.abs() < 1e-6 && r.abs() < 1e-6);

        let mut out = (0.0, 0.0);
        for _ in 0..10 {
            out = eng.process_sample(0.0, 0.0);
        }
        // After 10 more samples the impulse reaches the tap.
        let (lg, rg) = equal_power(0.0);
        assert!((out.0 - 0.5 * lg).abs() < 1e-5, "L {}", out.0);
        assert!((out.1 - 0.5 * rg).abs() < 1e-5, "R {}", out.1);
    }

    #[test]
    fn taps_sum() {
        // Two taps at the same time add their gains.
        let mut eng = settled_engine(&[Tap::new(5.0, 0.3, 0.0), Tap::new(5.0, 0.4, 0.0)], 1.0);
        let mut out = (0.0, 0.0);
        eng.process_sample(1.0, 1.0);
        for _ in 0..5 {
            out = eng.process_sample(0.0, 0.0);
        }
        let (lg, _) = equal_power(0.0);
        assert!((out.0 - 0.7 * lg).abs() < 1e-5, "got {}", out.0);
    }

    #[test]
    fn hard_pan_silences_other_side() {
        let mut eng = settled_engine(&[Tap::new(3.0, 1.0, -1.0)], 1.0);
        let mut out = (0.0, 0.0);
        eng.process_sample(1.0, 1.0);
        for _ in 0..3 {
            out = eng.process_sample(0.0, 0.0);
        }
        assert!((out.0 - 1.0).abs() < 1e-5, "L {}", out.0);
        assert!(out.1.abs() < 1e-6, "R {}", out.1);
    }

    #[test]
    fn negative_gain_inverts_polarity() {
        // Polarity (advanced): a negative tap gain flips the echo's sign, which
        // is what turns comb peaks into notches (design §5).
        let mut eng = settled_engine(&[Tap::new(4.0, -0.5, 0.0)], 1.0);
        let mut out = (0.0, 0.0);
        eng.process_sample(1.0, 1.0);
        for _ in 0..4 {
            out = eng.process_sample(0.0, 0.0);
        }
        let (lg, _) = equal_power(0.0);
        // Same magnitude as +0.5 would give, but inverted.
        assert!((out.0 - (-0.5 * lg)).abs() < 1e-5, "L {}", out.0);
        assert!(out.0 < 0.0, "expected inverted output, got {}", out.0);
    }

    #[test]
    fn mix_zero_is_dry() {
        let mut eng = settled_engine(&[Tap::new(2.0, 1.0, 0.0)], 0.0);
        let (l, r) = eng.process_sample(0.42, 0.42);
        assert!((l - 0.42).abs() < 1e-6 && (r - 0.42).abs() < 1e-6);
    }

    #[test]
    fn output_trim_scales() {
        let mut eng = settled_engine(&[], 0.0);
        eng.set_output_trim(0.5);
        eng.set_smoothing_ms(0.0);
        eng.set_output_trim(0.5);
        let (l, _) = eng.process_sample(1.0, 1.0);
        assert!((l - 0.5).abs() < 1e-6, "got {l}");
    }

    #[test]
    fn removed_tap_fades_out_without_click() {
        // Two short centered taps driven by DC; once settled, drop one tap and
        // confirm the output never jumps between consecutive samples (a click).
        let mut eng = Engine::new(SR, 4_096);
        eng.set_smoothing_ms(20.0);
        eng.set_mix(1.0);
        eng.set_output_trim(1.0);
        eng.set_taps(&[Tap::new(1.0, 1.0, 0.0), Tap::new(2.0, 1.0, 0.0)]);

        // Settle: fade-in completes and the delay line fills with DC.
        let mut prev = 0.0;
        for _ in 0..6_000 {
            prev = eng.process_sample(1.0, 1.0).0;
        }

        // Remove the second tap; the engine keeps it as a fading-out slot.
        eng.set_taps(&[Tap::new(1.0, 1.0, 0.0)]);
        assert_eq!(eng.num_taps(), 2, "removed tap is retained while fading");

        let mut max_jump: f32 = 0.0;
        // Run well past the ~20 ms fade so the gain reaches the GC threshold.
        for _ in 0..12_000 {
            let out = eng.process_sample(1.0, 1.0).0;
            max_jump = max_jump.max((out - prev).abs());
            prev = out;
        }
        // Abrupt removal would jump by ~0.7 in one sample; a fade keeps it tiny.
        assert!(max_jump < 0.01, "click on removal: max per-sample jump {max_jump}");

        // Once faded, a later update garbage-collects the dead slot.
        eng.set_taps(&[Tap::new(1.0, 1.0, 0.0)]);
        assert_eq!(eng.num_taps(), 1, "faded-out tap is dropped");
    }

    #[test]
    fn taps_beyond_buffer_do_not_pile_up() {
        // Reproduce "bump taps to max": ramp the count up to 128 with a tap
        // spacing large enough that most taps land beyond the buffer's max
        // delay, then drive an impulse and run past that delay. The too-long
        // taps must stay SILENT (read returns 0) rather than clamping onto the
        // max-delay position and firing together as one loud stack.
        let max_delay = 4_800;
        let mut eng = Engine::new(SR, max_delay);
        eng.reserve_taps(128);
        eng.set_smoothing_ms(0.0); // settle instantly for a clean level check
        eng.set_mix(1.0);
        eng.set_output_trim(1.0);

        let step = 1_000.0; // only taps 1..4 (≤4000) fit; 5..128 are too long
        let taps: Vec<Tap> = (0..128)
            .map(|i| Tap::new((i as f32 + 1.0) * step, 1.0, 0.0))
            .collect();
        eng.set_taps(&taps);

        // Impulse then silence, run well past max_delay.
        let mut max_abs: f32 = 0.0;
        for n in 0..20_000 {
            let x = if n < 1 { 1.0 } else { 0.0 };
            let (l, r) = eng.process_sample(x, x);
            assert!(l.is_finite() && r.is_finite(), "non-finite during playout");
            max_abs = max_abs.max(l.abs()).max(r.abs());
        }
        // Only the ~4 in-buffer taps ever sound (each ≈0.707 at centre, never
        // simultaneously), so the peak is ~1 — not the ~90-tap stack we'd get if
        // out-of-range taps clamped onto the same read position.
        assert!(max_abs < 2.0, "too-long taps piled up into a loud stack: {max_abs}");
    }

    #[test]
    fn limiter_bounds_summed_output() {
        // Eight centered taps at full gain stacked at the same delay sum to ~8x,
        // which clips hard. With the limiter on, the wet output stays bounded.
        let taps: Vec<Tap> = (0..8).map(|_| Tap::new(1.0, 1.0, 0.0)).collect();

        let mut without = settled_engine(&taps, 1.0);
        let mut with = settled_engine(&taps, 1.0);
        with.set_limiter_enabled(true);

        without.process_sample(1.0, 1.0);
        with.process_sample(1.0, 1.0);
        let (mut max_off, mut max_on): (f32, f32) = (0.0, 0.0);
        for _ in 0..200 {
            let off = without.process_sample(0.0, 0.0);
            let on = with.process_sample(0.0, 0.0);
            max_off = max_off.max(off.0.abs()).max(off.1.abs());
            max_on = max_on.max(on.0.abs()).max(on.1.abs());
        }
        assert!(max_off > 1.0, "unlimited sum should clip, got {max_off}");
        assert!(max_on <= 1.0, "limited sum should stay bounded, got {max_on}");
    }

    #[test]
    fn output_meter_tracks_peak_then_decays() {
        // One centered tap; drive a single impulse and confirm the meter rises
        // to the output peak, holds it, then decays toward zero.
        let mut eng = settled_engine(&[Tap::new(1.0, 1.0, 0.0)], 1.0);
        assert_eq!(eng.output_level(), 0.0);

        eng.process_sample(1.0, 1.0); // impulse in
        let after_impulse = eng.process_sample(0.0, 0.0).0; // tap fires here
        let peak = eng.output_level();
        assert!(peak > 0.0, "meter should register the peak");
        assert!((peak - after_impulse.abs()).abs() < 1e-6, "meter == |out|");

        // With silence in, the peak holds then decays but never rises again.
        // ~0.42 s at 48 kHz is well past the 0.3 s (1/e) release.
        let mut prev = peak;
        for _ in 0..20_000 {
            let now = eng.output_level();
            assert!(now <= prev + 1e-7, "meter must not rise without new signal");
            prev = now;
            eng.process_sample(0.0, 0.0);
        }
        assert!(prev < peak * 0.5, "meter should have decayed substantially");
    }

    #[test]
    fn appended_tap_fades_in_without_click() {
        let mut eng = Engine::new(SR, 4_096);
        eng.set_smoothing_ms(20.0);
        eng.set_mix(1.0);
        eng.set_taps(&[Tap::new(1.0, 1.0, 0.0)]);
        // Newly appended tap starts at gain 0 -> first output is ~silent.
        eng.process_sample(1.0, 1.0);
        let (l, _) = eng.process_sample(0.0, 0.0);
        assert!(l.abs() < 0.2, "should fade in, got {l}");
    }
}
