//! PR 11 — Plugin parameters.
//!
//! The host-facing toolbar controls (design §7) plus the persisted lane state.
//! Global controls are plain nih-plug params; the per-tap detach overrides live
//! in two `#[persist]` lanes so edits survive save/reload. Sources and tap
//! counts are pushed into the lanes from these params each block (PR 12); the
//! lanes' own state is only the link/detach overrides authored in the editor.

use delay_core::{Lane, LaneSource};
use nih_plug::prelude::*;
use parking_lot::RwLock;

/// Soft maximum tap count (design §7 — "soft max 128").
pub const MAX_TAPS: i32 = 128;

/// How tap spacing is specified.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum TimeMode {
    /// Tempo-synced note divisions.
    Sync,
    /// Free milliseconds.
    Free,
}

/// Note division used as the tap spacing in [`TimeMode::Sync`]. The value is in
/// quarter-note beats.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum NoteDivision {
    #[name = "1/1"]
    Whole,
    #[name = "1/2"]
    Half,
    #[name = "1/4 dotted"]
    DottedQuarter,
    #[name = "1/4"]
    Quarter,
    #[name = "1/4 triplet"]
    QuarterTriplet,
    #[name = "1/8 dotted"]
    DottedEighth,
    #[name = "1/8"]
    Eighth,
    #[name = "1/8 triplet"]
    EighthTriplet,
    #[name = "1/16"]
    Sixteenth,
    #[name = "1/16 triplet"]
    SixteenthTriplet,
}

impl NoteDivision {
    /// Length of this division in quarter-note beats.
    #[allow(dead_code)] // wired into process() in PR 12
    pub fn beats(self) -> f32 {
        match self {
            NoteDivision::Whole => 4.0,
            NoteDivision::Half => 2.0,
            NoteDivision::DottedQuarter => 1.5,
            NoteDivision::Quarter => 1.0,
            NoteDivision::QuarterTriplet => 2.0 / 3.0,
            NoteDivision::DottedEighth => 0.75,
            NoteDivision::Eighth => 0.5,
            NoteDivision::EighthTriplet => 1.0 / 3.0,
            NoteDivision::Sixteenth => 0.25,
            NoteDivision::SixteenthTriplet => 1.0 / 6.0,
        }
    }
}

/// Preset amplitude shape (the amplitude lane's source). The pan lane is driven
/// by the ping-pong generator, so it has no shape picker here.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum AmpShape {
    /// All taps at full amplitude.
    Flat,
    /// Exponential decay — the classic delay falloff.
    #[name = "Exp Decay"]
    ExpDecay,
    Sine,
    Saw,
    Triangle,
}

impl AmpShape {
    /// Map the shape + a normalized `amount` knob to a concrete [`LaneSource`].
    #[allow(dead_code)] // wired into process() in PR 12
    pub fn to_source(self, amount: f32) -> LaneSource {
        // Shapes that take a "cycles" parameter share this 0.5..4 mapping.
        let cycles = 0.5 + amount * 3.5;
        match self {
            AmpShape::Flat => LaneSource::Constant(1.0),
            AmpShape::ExpDecay => LaneSource::ExpDecay { k: amount * 6.0 },
            AmpShape::Sine => LaneSource::Sine { cycles, phase: 0.0 },
            AmpShape::Saw => LaneSource::Saw { cycles },
            AmpShape::Triangle => LaneSource::Triangle { cycles },
        }
    }
}

/// Default tap count, shared by the params and the initial lanes so they agree.
pub const DEFAULT_TAPS: i32 = 8;

#[derive(Params)]
pub struct DelayParams {
    #[id = "taps"]
    pub tap_count: IntParam,

    #[id = "timemode"]
    pub time_mode: EnumParam<TimeMode>,

    #[id = "division"]
    pub sync_division: EnumParam<NoteDivision>,

    #[id = "freems"]
    pub free_ms: FloatParam,

    #[id = "smoothing"]
    pub smoothing: FloatParam,

    #[id = "mix"]
    pub mix: FloatParam,

    #[id = "pingpong"]
    pub pingpong_amount: FloatParam,

    #[id = "output"]
    pub output_trim: FloatParam,

    #[id = "polarity"]
    pub polarity: BoolParam,

    #[id = "ampshape"]
    pub amp_shape: EnumParam<AmpShape>,

    #[id = "ampamount"]
    pub amp_amount: FloatParam,

    /// Persisted amplitude-lane edits (per-tap detach overrides). The source and
    /// count are overwritten from the params each block; only the link/detach
    /// state authored in the editor is meaningful to persist.
    #[persist = "amp_lane"]
    pub amp_lane: RwLock<Lane>,

    /// Persisted pan-lane edits.
    #[persist = "pan_lane"]
    pub pan_lane: RwLock<Lane>,
}

impl Default for DelayParams {
    fn default() -> Self {
        let taps = DEFAULT_TAPS as usize;
        Self {
            tap_count: IntParam::new("Taps", DEFAULT_TAPS, IntRange::Linear { min: 1, max: MAX_TAPS }),

            time_mode: EnumParam::new("Time Mode", TimeMode::Sync),
            sync_division: EnumParam::new("Division", NoteDivision::Eighth),
            free_ms: FloatParam::new(
                "Free Time",
                125.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 2000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            smoothing: FloatParam::new("Smoothing", 20.0, FloatRange::Linear { min: 0.0, max: 100.0 })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),

            mix: FloatParam::new("Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            pingpong_amount: FloatParam::new(
                "Ping-Pong",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            output_trim: FloatParam::new(
                "Output",
                0.0,
                FloatRange::Linear { min: -24.0, max: 24.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            polarity: BoolParam::new("Polarity", false),

            amp_shape: EnumParam::new("Amp Shape", AmpShape::ExpDecay),
            amp_amount: FloatParam::new("Amp Amount", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),

            amp_lane: RwLock::new(Lane::new(LaneSource::ExpDecay { k: 3.0 }, (0.0, 1.0), taps)),
            pan_lane: RwLock::new(Lane::new(
                LaneSource::PingPong { width: 0.5, widen: 0.0 },
                (-1.0, 1.0),
                taps,
            )),
        }
    }
}
