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
    /// `delay_samples` is clamped to `[0, max_delay()]`. Reading before the
    /// buffer has filled returns the zeros it was initialised with.
    #[inline]
    pub fn read(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let max = self.max_delay() as f32;
        let delay = delay_samples.clamp(0.0, max);

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
    fn delay_is_clamped_to_max() {
        let mut line = DelayLine::new(2);
        for v in [7.0, 8.0, 9.0] {
            line.write(v);
        }
        // max_delay == 2; asking for more clamps rather than panicking.
        let clamped = line.read(100.0);
        approx(clamped, line.read(2.0));
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
