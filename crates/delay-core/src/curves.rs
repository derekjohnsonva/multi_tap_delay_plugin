//! PR 8 — Preset curve shapes (design §2/§3 — "preset shapes: sine, saw, exp").
//!
//! Pure functions of a normalized position `x ∈ [0, 1]`, each returning a value
//! in roughly `[0, 1]`. They are the continuous shapes a lane's linked taps
//! sample; the lane clamps the result to its own range. Selecting a preset in
//! the editor is just [`crate::lane::Lane::set_source`] with the matching
//! [`crate::lane::LaneSource`] variant.

use core::f32::consts::TAU;

/// Sine shape mapped to `[0, 1]`: `0.5 + 0.5·sin(τ·(cycles·x + phase))`.
/// `cycles` is how many full periods span the lane; `phase` is in turns.
#[inline]
pub fn sine(x: f32, cycles: f32, phase: f32) -> f32 {
    0.5 + 0.5 * (TAU * (cycles * x + phase)).sin()
}

/// Rising sawtooth in `[0, 1)`: the fractional part of `cycles·x`.
#[inline]
pub fn saw(x: f32, cycles: f32) -> f32 {
    (cycles * x).rem_euclid(1.0)
}

/// Triangle in `[0, 1]`: 0 at the period edges, 1 at the midpoint.
#[inline]
pub fn triangle(x: f32, cycles: f32) -> f32 {
    let t = (cycles * x).rem_euclid(1.0);
    1.0 - (2.0 * t - 1.0).abs()
}

/// Exponential decay in `(0, 1]`: `exp(-k·x)`. `k = 0` is flat; larger `k`
/// decays faster. This is the classic delay falloff.
#[inline]
pub fn exp_decay(x: f32, k: f32) -> f32 {
    (-k * x).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn sine_known_points() {
        // One full cycle, no phase: 0.5 at x=0, peak at quarter, trough at 3/4.
        approx(sine(0.0, 1.0, 0.0), 0.5);
        approx(sine(0.25, 1.0, 0.0), 1.0);
        approx(sine(0.75, 1.0, 0.0), 0.0);
    }

    #[test]
    fn saw_ramps_and_wraps() {
        approx(saw(0.0, 1.0), 0.0);
        approx(saw(0.5, 1.0), 0.5);
        approx(saw(0.25, 2.0), 0.5); // two cycles -> twice as fast
    }

    #[test]
    fn triangle_peaks_at_midpoint() {
        approx(triangle(0.0, 1.0), 0.0);
        approx(triangle(0.5, 1.0), 1.0);
        approx(triangle(1.0, 1.0), 0.0);
    }

    #[test]
    fn exp_decay_falls_from_one() {
        approx(exp_decay(0.0, 3.0), 1.0);
        assert!(exp_decay(1.0, 3.0) < exp_decay(0.5, 3.0));
        assert!(exp_decay(0.5, 3.0) < 1.0);
    }

    #[test]
    fn exp_decay_zero_k_is_flat() {
        approx(exp_decay(0.0, 0.0), 1.0);
        approx(exp_decay(1.0, 0.0), 1.0);
    }
}
