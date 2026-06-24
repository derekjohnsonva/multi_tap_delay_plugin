//! `delay-core` — the DSP heart of the multi-tap delay.
//!
//! This crate is plain Rust with **no plugin/host dependency** so it stays
//! unit-testable without a DAW and can later compile to WASM for the web demo
//! (design doc §6). Phase 1 delivers the engine; the lane/curve model (Phase 2)
//! and parameter glue (Phase 3) build on top of it.

pub mod buffer;
pub mod curves;
pub mod engine;
pub mod lane;
pub mod limiter;
pub mod pan;
pub mod smoothing;

pub use buffer::DelayLine;
pub use engine::{Engine, Tap};
pub use lane::{Lane, LaneSource};
pub use limiter::Limiter;
pub use pan::equal_power;
pub use smoothing::OnePole;

/// Total delay (ms) below which the taps stop reading as discrete echoes and
/// comb-filter into a resonator/flanger, with the amplitude curve acting as a
/// spectral shaper (design §5). Not a limit — it's allowed and musical — but
/// the editor signposts this region on the time axis (design §7).
pub const COMB_ZONE_MS: f32 = 30.0;

/// Whether a tap at `delay_ms` falls in the comb zone (see [`COMB_ZONE_MS`]).
pub fn in_comb_zone(delay_ms: f32) -> bool {
    delay_ms < COMB_ZONE_MS
}

#[cfg(test)]
mod comb_tests {
    use super::*;

    #[test]
    fn comb_zone_covers_short_delays_only() {
        assert!(in_comb_zone(5.0));
        assert!(in_comb_zone(COMB_ZONE_MS - 0.1));
        assert!(!in_comb_zone(COMB_ZONE_MS));
        assert!(!in_comb_zone(250.0));
    }
}
