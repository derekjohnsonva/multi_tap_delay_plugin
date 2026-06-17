//! PR 5 — Per-coefficient smoothing.
//!
//! A one-pole smoother used on every per-tap coefficient (gain, pan) plus the
//! global mix so that parameter changes ramp instead of zippering (design doc
//! §4/§5). One sample of `next()` advances the ramp by one frame.

/// One-pole exponential smoother toward a target value.
#[derive(Clone)]
pub struct OnePole {
    current: f32,
    target: f32,
    /// Per-sample retention coefficient in `[0, 1)`. 0 == instantaneous.
    coeff: f32,
}

impl OnePole {
    /// Create a smoother already settled at `initial` with no smoothing.
    /// Call [`OnePole::set_time`] to choose a time constant.
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            coeff: 0.0,
        }
    }

    /// Set the smoothing time constant. `time_ms == 0` disables smoothing.
    ///
    /// `coeff = exp(-1 / (tau_samples))`, so after `time_ms` the ramp has
    /// covered ~63% of the distance to a new target (standard one-pole).
    pub fn set_time(&mut self, time_ms: f32, sample_rate: f32) {
        if time_ms <= 0.0 || sample_rate <= 0.0 {
            self.coeff = 0.0;
            return;
        }
        let tau_samples = (time_ms / 1000.0) * sample_rate;
        self.coeff = (-1.0 / tau_samples).exp();
    }

    /// Aim the smoother at a new value; `next()` will ramp toward it.
    #[inline]
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump immediately to `value` (no ramp). Use on reset / first block.
    pub fn set_immediate(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }

    /// Current settled/ramping value without advancing.
    #[inline]
    pub fn value(&self) -> f32 {
        self.current
    }

    /// True once the ramp has effectively reached its target.
    ///
    /// The threshold is `1e-4` (≈ −80 dB), not machine epsilon: a one-pole
    /// recurrence stalls at a residual of roughly `0.5·ULP / (1 - coeff)`,
    /// which for slow smoothing sits well above `1e-6`. `1e-4` is inaudible
    /// and reliably reachable.
    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < 1e-4
    }

    /// Advance one sample and return the new current value.
    ///
    /// Named `next` to mirror nih-plug's smoother convention; this is not an
    /// `Iterator`.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        self.current = self.target + (self.current - self.target) * self.coeff;
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_time_is_instant() {
        let mut s = OnePole::new(0.0);
        s.set_time(0.0, 48_000.0);
        s.set_target(1.0);
        assert_eq!(s.next(), 1.0);
    }

    #[test]
    fn step_ramps_rather_than_jumps() {
        let mut s = OnePole::new(0.0);
        s.set_time(10.0, 48_000.0);
        s.set_target(1.0);
        let first = s.next();
        // Moves toward target but nowhere near arriving in one sample.
        assert!(first > 0.0 && first < 0.1, "got {first}");
    }

    #[test]
    fn reaches_63_percent_after_one_time_constant() {
        let sr = 48_000.0;
        let ms = 10.0;
        let mut s = OnePole::new(0.0);
        s.set_time(ms, sr);
        s.set_target(1.0);
        let tau_samples = ((ms / 1000.0) * sr) as usize;
        for _ in 0..tau_samples {
            s.next();
        }
        // Within a couple percent of the canonical 1 - 1/e ≈ 0.632.
        assert!((s.value() - 0.632).abs() < 0.02, "got {}", s.value());
    }

    #[test]
    fn eventually_settles() {
        let mut s = OnePole::new(0.0);
        s.set_time(5.0, 48_000.0);
        s.set_target(0.75);
        for _ in 0..48_000 {
            s.next();
        }
        assert!(s.is_settled());
        assert!((s.value() - 0.75).abs() < 1e-3);
    }
}
