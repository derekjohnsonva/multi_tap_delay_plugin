//! PR 1 — Loadable stereo passthrough plugin.
//!
//! This is the scaffolding milestone: a CLAP + VST3 plugin that loads in a host
//! and passes stereo audio through unchanged. No delay, no parameters, no GUI
//! yet — those arrive in later phases (DSP engine wiring in Phase 3, egui editor
//! in Phase 4). It exists to prove the build/bundle/load pipeline end-to-end.

mod params;

use nih_plug::prelude::*;
use params::DelayParams;
use std::sync::Arc;

struct DelayPlugin {
    params: Arc<DelayParams>,
}

impl Default for DelayPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(DelayParams::default()),
        }
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

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Passthrough: leave the buffer untouched.
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
