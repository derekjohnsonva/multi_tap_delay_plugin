//! PR 4 — Equal-power pan law.
//!
//! Maps a pan position `-1.0` (hard left) .. `+1.0` (hard right) to a pair of
//! channel gains whose squared sum is unity, so perceived loudness is constant
//! across the sweep and the center sits at −3 dB (design doc §4/§8).

use core::f32::consts::FRAC_PI_2;

/// Return `(left_gain, right_gain)` for a pan position in `[-1, 1]`.
#[inline]
pub fn equal_power(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    // Map [-1, 1] -> [0, PI/2]. angle 0 => hard left, PI/2 => hard right.
    let angle = (p + 1.0) * 0.5 * FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn center_is_minus_3db() {
        let (l, r) = equal_power(0.0);
        approx(l, std::f32::consts::FRAC_1_SQRT_2);
        approx(r, std::f32::consts::FRAC_1_SQRT_2);
    }

    #[test]
    fn hard_left_and_right() {
        let (l, r) = equal_power(-1.0);
        approx(l, 1.0);
        approx(r, 0.0);
        let (l, r) = equal_power(1.0);
        approx(l, 0.0);
        approx(r, 1.0);
    }

    #[test]
    fn power_sums_to_unity_across_sweep() {
        for i in 0..=20 {
            let pan = -1.0 + i as f32 * 0.1;
            let (l, r) = equal_power(pan);
            approx(l * l + r * r, 1.0);
        }
    }

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(equal_power(-5.0), equal_power(-1.0));
        assert_eq!(equal_power(5.0), equal_power(1.0));
    }
}
