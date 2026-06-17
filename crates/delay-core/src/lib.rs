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
pub mod pan;
pub mod smoothing;

pub use buffer::DelayLine;
pub use engine::{Engine, Tap};
pub use lane::{Lane, LaneSource};
pub use pan::equal_power;
pub use smoothing::OnePole;
