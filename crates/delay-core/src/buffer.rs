//! PR 2 — Circular delay buffer with fractional read.
//!
//! A single-channel circular buffer. `write` pushes one sample; `read` looks
//! back `delay_samples` (a real number) using linear interpolation between the
//! two neighbouring stored samples. Linear interpolation is sufficient here
//! because we only modulate *amplitude*, never tap *time*, so there is no
//! Doppler/pitch artifact to worry about (design doc §5).

/// A power-of-nothing circular delay line. Allocate once with [`DelayLine::new`]
/// and reuse; it never reallocates during processing.
pub struct DelayLine {
    buffer: Vec<f32>,
    /// Index the *next* written sample will occupy.
    write_pos: usize,
}

impl DelayLine {
    /// Create a delay line that can look back up to `max_delay_samples`.
    ///
    /// Two extra slots are allocated so that interpolation at the maximum delay
    /// (which reads `floor(delay)` and `floor(delay) + 1`) stays in bounds.
    pub fn new(max_delay_samples: usize) -> Self {
        let len = max_delay_samples.max(1) + 2;
        Self {
            buffer: vec![0.0; len],
            write_pos: 0,
        }
    }

    /// Largest delay (in samples) that can be read back with interpolation.
    pub fn max_delay(&self) -> usize {
        self.buffer.len() - 2
    }

    /// Zero the buffer and reset the write head. Call on `reset()`/transport
    /// changes to avoid bleeding stale audio.
    pub fn reset(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
        self.write_pos = 0;
    }

    /// Push one sample. The most recently written sample is the one read back
    /// at `delay_samples == 0.0`.
    #[inline]
    pub fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos += 1;
        if self.write_pos == self.buffer.len() {
            self.write_pos = 0;
        }
    }

    /// Read the sample written `delay_samples` ago, linearly interpolated.
    ///
    /// A delay **beyond** `max_delay()` returns silence (`0.0`): that sample is
    /// older than the buffer can hold, so it genuinely isn't there. Clamping it
    /// to the maximum instead would make every too-long tap pile up on the same
    /// read position — a loud stack of simultaneous echoes. Negative delays read
    /// the newest sample, and a non-finite delay (NaN/Inf) is treated as silence
    /// rather than propagating into the output. Reads before the buffer has
    /// filled return the zeros it was initialised with.
    #[inline]
    pub fn read(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let max = self.max_delay() as f32;
        // Too old to hold, or non-finite: silence (NaN must not propagate).
        if delay_samples.is_nan() || delay_samples > max {
            return 0.0;
        }
        let delay = delay_samples.max(0.0);

        let d_floor = delay.floor();
        let frac = delay - d_floor;
        let i0 = d_floor as usize; // newer neighbour
        let i1 = i0 + 1; // older neighbour

        // Most recent sample sits at write_pos - 1; walk backwards from there.
        // Bias by 2*len so the subtraction never underflows usize (i1 <= len-1).
        let base = self.write_pos + 2 * len - 1;
        let idx0 = (base - i0) % len;
        let idx1 = (base - i1) % len;

        let s0 = self.buffer[idx0];
        let s1 = self.buffer[idx1];
        s0 + (s1 - s0) * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn integer_delays_are_exact() {
        let mut line = DelayLine::new(8);
        // Write 1,2,3,4,5; most recent is 5.0.
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            line.write(v);
        }
        approx(line.read(0.0), 5.0);
        approx(line.read(1.0), 4.0);
        approx(line.read(2.0), 3.0);
        approx(line.read(3.0), 2.0);
        approx(line.read(4.0), 1.0);
    }

    #[test]
    fn fractional_delays_interpolate() {
        let mut line = DelayLine::new(8);
        for v in [10.0, 20.0] {
            line.write(v);
        }
        // delay 0 -> 20, delay 1 -> 10; halfway is the mean.
        approx(line.read(0.5), 15.0);
        approx(line.read(0.25), 17.5);
    }

    #[test]
    fn reads_before_fill_are_silent() {
        let line = DelayLine::new(8);
        approx(line.read(0.0), 0.0);
        approx(line.read(3.7), 0.0);
    }

    #[test]
    fn wraps_around_correctly() {
        let mut line = DelayLine::new(4); // capacity 4 + 1 slot
        // Write more than capacity; only the last few survive.
        for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
            line.write(v);
        }
        approx(line.read(0.0), 6.0);
        approx(line.read(1.0), 5.0);
        approx(line.read(4.0), 2.0);
    }

    #[test]
    fn reads_beyond_max_are_silent() {
        let mut line = DelayLine::new(2);
        for v in [7.0, 8.0, 9.0] {
            line.write(v);
        }
        // The oldest holdable sample still reads...
        approx(line.read(2.0), 7.0);
        // ...but anything older than the buffer is silence, NOT clamped onto the
        // max-delay sample (which would pile up too-long taps into a loud stack).
        approx(line.read(2.1), 0.0);
        approx(line.read(100.0), 0.0);
    }

    #[test]
    fn non_finite_delay_is_silent() {
        let mut line = DelayLine::new(8);
        for v in [1.0, 2.0, 3.0] {
            line.write(v);
        }
        // A NaN/Inf delay must not propagate into the output.
        approx(line.read(f32::NAN), 0.0);
        approx(line.read(f32::INFINITY), 0.0);
    }

    #[test]
    fn negative_delay_reads_newest() {
        let mut line = DelayLine::new(8);
        for v in [1.0, 2.0, 3.0] {
            line.write(v);
        }
        approx(line.read(-5.0), 3.0);
    }

    #[test]
    fn reset_clears_audio() {
        let mut line = DelayLine::new(4);
        for v in [1.0, 2.0, 3.0] {
            line.write(v);
        }
        line.reset();
        approx(line.read(0.0), 0.0);
        approx(line.read(2.0), 0.0);
    }
}
