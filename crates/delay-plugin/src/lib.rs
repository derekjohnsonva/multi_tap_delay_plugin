//! Multi-tap delay plugin (CLAP + VST3).
//!
//! PR 12 wires the parameters (PR 11) into the `delay-core` engine: each block
//! we translate params into tap delay times (ms or tempo-synced divisions),
//! sample the amplitude/pan lanes for per-tap gain/pan, and hand the taps to
//! the engine, which smooths every coefficient. The GUI (Phase 4) edits the
//! same lanes; here they're driven entirely from the params.

mod editor;
mod params;

use delay_core::{Engine, Tap};
use nih_plug::prelude::*;
use params::{DelayParams, TimeMode, MAX_TAPS};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Longest tap time the delay buffer can hold; taps scheduled past this go
/// silent (they can't be stored). Generous enough that high tap counts at
/// musical tempos are usable — 30 s holds, e.g., 128 × 1/16 at 120 BPM. At
/// 48 kHz stereo this is ~11.5 MB. Tunable: larger costs proportional memory.
const MAX_DELAY_SECONDS: f32 = 30.0;
/// BPM used when the host reports no tempo (standalone / stopped transport).
const FALLBACK_BPM: f32 = 120.0;

struct DelayPlugin {
    params: Arc<DelayParams>,
    engine: Engine,
    sample_rate: f32,
    /// Reused per-block tap buffer so `process()` never allocates.
    scratch: Vec<Tap>,
    /// Last tempo seen by the audio thread, shared with the editor so it can
    /// render sync-mode tap times in ms (e.g. the comb-zone hint, PR 17).
    current_bpm: Arc<AtomicF32>,
    /// Post-trim output level (linear peak), published for the editor's output
    /// meter (PR 18).
    meter_level: Arc<AtomicF32>,
}

impl Default for DelayPlugin {
    fn default() -> Self {
        // The engine is rebuilt in `initialize()` once the real sample rate is
        // known; this placeholder keeps `Default` total.
        Self {
            params: Arc::new(DelayParams::default()),
            engine: Engine::new(44_100.0, 44_100),
            sample_rate: 44_100.0,
            scratch: Vec::with_capacity(MAX_TAPS as usize),
            current_bpm: Arc::new(AtomicF32::new(FALLBACK_BPM)),
            meter_level: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl DelayPlugin {
    /// Tap spacing (samples between consecutive taps) for the current params.
    fn step_samples(&self, bpm: f32) -> f32 {
        match self.params.time_mode.value() {
            TimeMode::Sync => {
                let beats = self.params.sync_division.value().beats();
                beats * (60.0 / bpm) * self.sample_rate
            }
            TimeMode::Free => self.params.free_ms.value() / 1000.0 * self.sample_rate,
        }
    }

    /// Rebuild the engine's tap set from the params + current tempo. Reads the
    /// persisted lanes non-blockingly; if the editor holds the lock this block
    /// is skipped and the engine keeps smoothing toward its last targets.
    fn update_taps(&mut self, bpm: f32) {
        let count = self.params.tap_count.value() as usize;
        let step = self.step_samples(bpm);
        let amp_source = self
            .params
            .amp_shape
            .value()
            .to_source(self.params.amp_amount.value());
        let pan_source = delay_core::LaneSource::PingPong {
            width: self.params.pingpong_amount.value(),
            widen: 0.0,
        };
        // Polarity (PR 13) widens the amplitude lane to bipolar.
        let amp_range = if self.params.polarity.value() {
            (-1.0, 1.0)
        } else {
            (0.0, 1.0)
        };

        let (Some(mut amp), Some(mut pan)) =
            (self.params.amp_lane.try_write(), self.params.pan_lane.try_write())
        else {
            return;
        };

        amp.set_range(amp_range.0, amp_range.1);
        amp.set_source(amp_source);
        amp.set_count(count);
        // Pan is bipolar (hard L .. hard R). The lane's range must be re-applied
        // every block: a persisted pan lane deserializes with a placeholder
        // unipolar (0, 1) range, which would clamp every left tap to centre and
        // leave only right-panned taps audible.
        pan.set_range(-1.0, 1.0);
        pan.set_source(pan_source);
        pan.set_count(count);

        self.scratch.clear();
        for i in 0..count {
            let delay = (i as f32 + 1.0) * step;
            self.scratch.push(Tap::new(delay, amp.value(i), pan.value(i)));
        }
        drop(amp);
        drop(pan);

        self.engine.set_taps(&self.scratch);
    }
}

impl Plugin for DelayPlugin {
    const NAME: &'static str = "Multi-tap Delay";
    const VENDOR: &'static str = "delay_plugin";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.current_bpm.clone(),
            self.meter_level.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let max_delay = (self.sample_rate * MAX_DELAY_SECONDS).ceil() as usize;
        self.engine = Engine::new(self.sample_rate, max_delay);
        self.engine.set_smoothing_ms(self.params.smoothing.value());
        // Pre-allocate so neither the scratch nor the engine's tap set allocates
        // on the audio thread when the tap count is ramped up to the max.
        self.scratch.reserve(MAX_TAPS as usize);
        self.engine.reserve_taps(MAX_TAPS as usize);
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Per-block config: rebuild taps and push the scalar params.
        let bpm = context.transport().tempo.unwrap_or(FALLBACK_BPM as f64) as f32;
        self.current_bpm.store(bpm, Ordering::Relaxed);
        self.update_taps(bpm);
        self.engine.set_smoothing_ms(self.params.smoothing.value());
        self.engine.set_mix(self.params.mix.value());
        self.engine
            .set_output_trim(util::db_to_gain(self.params.output_trim.value()));
        self.engine
            .set_limiter_enabled(self.params.limiter.value());

        // Stereo in-place processing. Our only IO layout is stereo, but guard
        // the channel count so a mono host config can't panic.
        let channels = buffer.as_slice();
        if channels.len() >= 2 {
            let (left, right) = channels.split_at_mut(1);
            self.engine.process(left[0], right[0]);
        } else if let [mono] = channels {
            for sample in mono.iter_mut() {
                let (l, _r) = self.engine.process_sample(*sample, *sample);
                *sample = l;
            }
        }

        // Publish the post-trim output level for the editor's meter.
        self.meter_level
            .store(self.engine.output_level(), Ordering::Relaxed);

        ProcessStatus::Normal
    }
}

impl ClapPlugin for DelayPlugin {
    const CLAP_ID: &'static str = "com.delay-plugin.multitap-delay";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Multi-tap delay with per-tap amplitude and pan curves");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Delay,
    ];
}

impl Vst3Plugin for DelayPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"MultiTapDelay001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Delay];
}

nih_export_clap!(DelayPlugin);
nih_export_vst3!(DelayPlugin);
