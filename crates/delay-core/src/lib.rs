//! `delay-core` — the DSP heart of the multi-tap delay.
//!
//! This crate is plain Rust with **no plugin/host dependency** so it stays
//! unit-testable without a DAW and can later compile to WASM for the web demo
//! (design doc §6). Phase 1 fills in the engine module by module.

pub mod buffer;
pub mod pan;

pub use buffer::DelayLine;
pub use pan::equal_power;
