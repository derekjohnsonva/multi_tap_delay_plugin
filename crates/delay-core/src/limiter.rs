//! PR 20 — Optional safety limiter on the wet path (design §4).
//!
//! Summed taps exceed 0 dBFS easily even when every tap is ≤ 1, so the wet sum
//! can clip. This is an opt-in safety net, not a sound-design tool: a
//! feed-forward peak limiter that strictly caps the louder of the two channels
//! to a ceiling just under 0 dBFS.
//!
//! Attack is instantaneous (the gain drops to exactly cancel the overshoot the
//! same sample, so the output never exceeds the ceiling — no lookahead, no added
//! latency), while release ramps the gain back up over a time constant so gain
//! changes don't pump or click. When disabled it is a transparent passthrough.

/// Linear ceiling the limiter holds the wet signal under (≈ −0.1 dBFS).
const CEILING: f32 = 0.99;

/// Time constant (seconds) for the gain to recover after a peak.
const RELEASE_SECONDS: f32 = 0.1;

/// A stereo feed-forward safety limiter. Allocate once; per-sample work is
/// allocation-free.
pub struct Limiter {
    enabled: bool,
    /// Current gain reduction, `1.0` = no reduction.
    gain: f32,
    /// Per-sample release coefficient (one-pole) toward `1.0`.
    release: f32,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            enabled: false,
            gain: 1.0,
            release: (-1.0 / (RELEASE_SECONDS * sample_rate)).exp(),
        }
    }

    /// Recompute the release coefficient for a new sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.release = (-1.0 / (RELEASE_SECONDS * sample_rate)).exp();
    }

    /// Enable or disable limiting. Disabling snaps the gain back to unity so the
    /// next enable starts clean.
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.gain = 1.0;
        }
        self.enabled = enabled;
    }

    /// Snap to unity gain (e.g. on transport reset).
    pub fn reset(&mut self) {
        self.gain = 1.0;
    }

    /// Limit one stereo frame. Returns the input unchanged when disabled.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }
        let peak = l.abs().max(r.abs());
        let target = if peak > CEILING { CEILING / peak } else { 1.0 };
        if target < self.gain {
            // Instantaneous attack: cancel the overshoot this very sample.
            self.gain = target;
        } else {
            // Smooth release back up toward the target.
            self.gain = target + (self.gain - target) * self.release;
        }
        (l * self.gain, r * self.gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn disabled_is_transparent() {
        let mut lim = Limiter::new(SR); // disabled by default
        for &x in &[0.0, 0.5, 2.0, -3.0, 10.0] {
            assert_eq!(lim.process(x, -x), (x, -x));
        }
    }

    #[test]
    fn caps_output_to_ceiling() {
        let mut lim = Limiter::new(SR);
        lim.set_enabled(true);
        // Loud, varying input: output magnitude never exceeds the ceiling.
        let mut max_out: f32 = 0.0;
        for n in 0..10_000 {
            let x = 3.0 * ((n as f32) * 0.01).sin();
            let (l, r) = lim.process(x, 0.5 * x);
            max_out = max_out.max(l.abs()).max(r.abs());
        }
        assert!(max_out <= CEILING + 1e-6, "exceeded ceiling: {max_out}");
    }

    #[test]
    fn leaves_quiet_signal_alone() {
        let mut lim = Limiter::new(SR);
        lim.set_enabled(true);
        // Below the ceiling the gain stays at unity.
        let (l, r) = lim.process(0.5, -0.3);
        assert!((l - 0.5).abs() < 1e-6 && (r + 0.3).abs() < 1e-6);
    }

    #[test]
    fn gain_releases_after_a_peak() {
        let mut lim = Limiter::new(SR);
        lim.set_enabled(true);
        // A loud sample pulls the gain down...
        lim.process(5.0, 0.0);
        let reduced = lim.gain;
        assert!(reduced < 1.0);
        // ...and quiet samples let it recover toward unity. ~1 s is many
        // release time constants (0.1 s each), so it returns essentially to 1.0.
        for _ in 0..50_000 {
            lim.process(0.1, 0.1);
        }
        assert!(lim.gain > reduced, "gain should recover");
        assert!(lim.gain > 0.99, "gain should be near unity after release");
    }
}
