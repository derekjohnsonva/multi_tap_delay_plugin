//! PR 7 — Lane abstraction (the spine of the design, §3).
//!
//! A **lane** is a continuous/generated source + `N` discrete taps that sample
//! it + per-tap detach overrides. Each tap is either:
//!
//! - **linked** — its value is `source.value(index, count)`, so it follows the
//!   source live (editing the source moves every linked tap at once), or
//! - **detached** — its value is a stored override that ignores the source.
//!
//! Amplitude and Pan are both lanes; this one model gives draw-a-shape editing,
//! per-tap tweaking, preset shapes, and ping-pong without special cases.
//!
//! Preset curve shapes arrive in PR 8 and the ping-pong generator in PR 9 (both
//! extend [`LaneSource`]); the tap-count change rule arrives in PR 10.

use crate::curves;

/// What feeds a lane's *linked* taps. Continuous shapes are sampled at a
/// normalized x = `index / (count - 1)`; index-based generators (ping-pong)
/// use the index directly. The ping-pong generator is added in PR 9.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneSource {
    /// Every tap gets the same value.
    Constant(f32),
    /// Linear ramp from `start` (first tap) to `end` (last tap).
    Ramp { start: f32, end: f32 },
    /// Sine shape (`cycles` periods across the lane, `phase` in turns).
    Sine { cycles: f32, phase: f32 },
    /// Rising sawtooth (`cycles` periods across the lane).
    Saw { cycles: f32 },
    /// Triangle (`cycles` periods across the lane).
    Triangle { cycles: f32 },
    /// Exponential decay `exp(-k·x)` — the classic delay falloff.
    ExpDecay { k: f32 },
}

impl LaneSource {
    /// Normalized position of tap `index` within `count` taps, in `[0, 1]`.
    /// A single tap sits at the start of the source.
    #[inline]
    pub fn x_of(index: usize, count: usize) -> f32 {
        if count <= 1 {
            0.0
        } else {
            index as f32 / (count - 1) as f32
        }
    }

    /// Raw (unclamped) value this source assigns to tap `index` of `count`.
    pub fn value(&self, index: usize, count: usize) -> f32 {
        let x = Self::x_of(index, count);
        match *self {
            LaneSource::Constant(v) => v,
            LaneSource::Ramp { start, end } => start + (end - start) * x,
            LaneSource::Sine { cycles, phase } => curves::sine(x, cycles, phase),
            LaneSource::Saw { cycles } => curves::saw(x, cycles),
            LaneSource::Triangle { cycles } => curves::triangle(x, cycles),
            LaneSource::ExpDecay { k } => curves::exp_decay(x, k),
        }
    }
}

/// Per-tap link state.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LinkState {
    /// Follows the source live.
    Linked,
    /// Frozen at a stored override value.
    Detached(f32),
}

/// A parameter lane: a source plus `N` taps that sample it.
#[derive(Clone, Debug)]
pub struct Lane {
    source: LaneSource,
    /// Resolved values are clamped to this inclusive range.
    min: f32,
    max: f32,
    taps: Vec<LinkState>,
}

impl Lane {
    /// Create a lane with `count` linked taps, clamped to `range`.
    pub fn new(source: LaneSource, range: (f32, f32), count: usize) -> Self {
        Self {
            source,
            min: range.0,
            max: range.1,
            taps: vec![LinkState::Linked; count],
        }
    }

    /// Number of taps.
    pub fn count(&self) -> usize {
        self.taps.len()
    }

    /// The current source.
    pub fn source(&self) -> LaneSource {
        self.source
    }

    /// Replace the source. Linked taps immediately follow the new source;
    /// detached taps keep their overrides. (Design §3 — "editing the curve
    /// moves all linked taps at once".)
    pub fn set_source(&mut self, source: LaneSource) {
        self.source = source;
    }

    /// Whether tap `index` is linked to the source.
    pub fn is_linked(&self, index: usize) -> bool {
        matches!(self.taps.get(index), Some(LinkState::Linked))
    }

    /// Resolved value of tap `index`, clamped to the lane range. Out-of-range
    /// indices return the clamped source value at that index.
    pub fn value(&self, index: usize) -> f32 {
        let raw = match self.taps.get(index) {
            Some(LinkState::Detached(v)) => *v,
            _ => self.source.value(index, self.count()),
        };
        raw.clamp(self.min, self.max)
    }

    /// Resolved values for every tap, in order. This is what the engine reads.
    pub fn values(&self) -> Vec<f32> {
        (0..self.count()).map(|i| self.value(i)).collect()
    }

    /// Detach tap `index`, freezing it at its current resolved value so it
    /// doesn't jump. No-op if already detached or out of range.
    pub fn detach(&mut self, index: usize) {
        if index >= self.count() {
            return;
        }
        if matches!(self.taps[index], LinkState::Linked) {
            let frozen = self.value(index);
            self.taps[index] = LinkState::Detached(frozen);
        }
    }

    /// Relink tap `index` so it follows the source again. No-op if out of range.
    pub fn relink(&mut self, index: usize) {
        if let Some(slot) = self.taps.get_mut(index) {
            *slot = LinkState::Linked;
        }
    }

    /// Set tap `index` to an explicit value, detaching it if needed. This is
    /// what dragging a tap in the editor calls. No-op if out of range.
    pub fn set_tap_value(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.taps.get_mut(index) {
            *slot = LinkState::Detached(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn linked_taps_follow_the_source() {
        let lane = Lane::new(LaneSource::Ramp { start: 0.0, end: 1.0 }, (0.0, 1.0), 5);
        approx(lane.value(0), 0.0);
        approx(lane.value(2), 0.5);
        approx(lane.value(4), 1.0);
    }

    #[test]
    fn single_tap_samples_source_start() {
        let lane = Lane::new(LaneSource::Ramp { start: 0.2, end: 0.9 }, (0.0, 1.0), 1);
        approx(lane.value(0), 0.2);
    }

    #[test]
    fn detach_freezes_current_value() {
        let mut lane = Lane::new(LaneSource::Ramp { start: 0.0, end: 1.0 }, (0.0, 1.0), 5);
        lane.detach(2);
        approx(lane.value(2), 0.5); // frozen at what it was
        assert!(!lane.is_linked(2));
        // Changing the source no longer moves it.
        lane.set_source(LaneSource::Constant(0.1));
        approx(lane.value(2), 0.5);
        // ...but linked neighbours do move.
        approx(lane.value(0), 0.1);
    }

    #[test]
    fn relink_resamples() {
        let mut lane = Lane::new(LaneSource::Ramp { start: 0.0, end: 1.0 }, (0.0, 1.0), 5);
        lane.detach(2);
        lane.set_source(LaneSource::Constant(0.3));
        approx(lane.value(2), 0.5);
        lane.relink(2);
        approx(lane.value(2), 0.3); // back on the source
        assert!(lane.is_linked(2));
    }

    #[test]
    fn editing_source_moves_linked_leaves_detached() {
        let mut lane = Lane::new(LaneSource::Constant(0.5), (0.0, 1.0), 4);
        lane.set_tap_value(1, 0.8); // detach via explicit set
        lane.set_source(LaneSource::Constant(0.2));
        approx(lane.value(0), 0.2);
        approx(lane.value(1), 0.8); // detached override held
        approx(lane.value(2), 0.2);
    }

    #[test]
    fn values_are_clamped_to_range() {
        // Pan-style bipolar range; a ramp that overshoots gets clamped.
        let lane = Lane::new(LaneSource::Ramp { start: -2.0, end: 2.0 }, (-1.0, 1.0), 3);
        approx(lane.value(0), -1.0);
        approx(lane.value(1), 0.0);
        approx(lane.value(2), 1.0);
    }

    #[test]
    fn selecting_a_preset_resamples_linked_keeps_detached() {
        // Exponential decay over 5 taps; detach the middle one.
        let mut lane = Lane::new(LaneSource::ExpDecay { k: 2.0 }, (0.0, 1.0), 5);
        approx(lane.value(0), 1.0); // exp(-0) == 1
        lane.set_tap_value(2, 0.42);
        // Switch to a different preset: linked taps re-sample, detached holds.
        lane.set_source(LaneSource::Constant(0.7));
        approx(lane.value(0), 0.7);
        approx(lane.value(4), 0.7);
        approx(lane.value(2), 0.42);
    }

    #[test]
    fn shape_sources_sample_at_tap_positions() {
        // Triangle over 5 taps -> x = 0, .25, .5, .75, 1; peak at the middle.
        let lane = Lane::new(LaneSource::Triangle { cycles: 1.0 }, (0.0, 1.0), 5);
        approx(lane.value(0), 0.0);
        approx(lane.value(2), 1.0);
        approx(lane.value(4), 0.0);
    }

    #[test]
    fn set_tap_value_detaches() {
        let mut lane = Lane::new(LaneSource::Constant(0.5), (0.0, 1.0), 3);
        assert!(lane.is_linked(1));
        lane.set_tap_value(1, 0.25);
        assert!(!lane.is_linked(1));
        approx(lane.value(1), 0.25);
    }
}
